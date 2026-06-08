use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::DeviceTrait;
use crossbeam_channel::bounded;
use rtrb::RingBuffer;
use tracing::{debug, error, info};

use crate::audio::{capture, device, playback};
use crate::config::AppConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::pipeline::shutdown::ShutdownSignal;
use crate::stt::gigaam::GigaAmStt;
use crate::translation::engine::TranslationEngine;
use crate::vad::detector::VadDetector;

#[derive(Debug, Clone, Copy)]
pub enum PipelineMode {
    Passthrough,
    SttOnly,
    Full,
}

pub struct Orchestrator {
    config: AppConfig,
    mode: PipelineMode,
    shutdown: ShutdownSignal,
}

impl Orchestrator {
    pub fn new(config: AppConfig, mode: PipelineMode) -> Self {
        let shutdown = ShutdownSignal::new();
        Orchestrator { config, mode, shutdown }
    }

    pub fn run(&self) -> Result<()> {
        match self.mode {
            PipelineMode::Passthrough => self.run_passthrough(),
            PipelineMode::SttOnly => self.run_stt_only(),
            PipelineMode::Full => self.run_full(),
        }
    }

    fn run_passthrough(&self) -> Result<()> {
        info!("Starting passthrough mode (mic → speaker)");
        let sr = self.config.audio.sample_rate;
        let ch = self.config.audio.channels;
        let cap = self.config.audio.ring_buffer_capacity;

        let (prod, cons) = RingBuffer::<f32>::new(cap);
        let in_dev = device::default_input_device()?;
        let out_dev = device::default_output_device()?;
        info!("Input:  {}", in_dev.name().unwrap_or_default());
        info!("Output: {}", out_dev.name().unwrap_or_default());

        let _in_stream = capture::start_capture(&in_dev, sr, ch, prod)?;
        let _out_stream = playback::start_playback(&out_dev, sr, ch, cons)?;

        info!("Passthrough active. Press Ctrl+C to stop.");
        self.wait_for_shutdown();
        info!("Passthrough stopped.");
        Ok(())
    }

    fn run_stt_only(&self) -> Result<()> {
        info!("Starting streaming STT mode");
        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;

        let (producer, mut consumer) = RingBuffer::<f32>::new(sample_rate as usize * 8);
        let input_device = device::default_input_device()?;
        let _input_stream = capture::start_capture_converted(&input_device, sample_rate, producer)?;

        let shutdown = self.shutdown.clone();
        let stt_cfg = self.config.stt.clone();
        let vad_cfg = self.config.vad.clone();

        let _handle = thread::Builder::new().name("stt".into()).spawn(move || {
            let stt = match GigaAmStt::new(&stt_cfg) {
                Ok(s) => s, Err(e) => { error!("STT: {e}"); return; }
            };
            let vad = match VadDetector::new(&vad_cfg, sample_rate) {
                Ok(v) => v, Err(e) => { error!("VAD: {e}"); return; }
            };
            info!("STT + Silero VAD ready");
            println!("--- говорите ---");

            let partial_every = sample_rate as usize;
            let chunk = 512usize;
            let mut tmp = vec![0.0f32; chunk];
            let mut pbuf: Vec<f32> = Vec::new();
            let mut since_partial = 0usize;

            while !shutdown.is_shutdown() {
                let avail = consumer.slots();
                if avail < chunk { thread::sleep(Duration::from_millis(5)); continue; }
                let n = avail.min(chunk);
                for i in 0..n { tmp[i] = consumer.pop().unwrap_or(0.0); }
                let frame = &tmp[..n];
                vad.accept_waveform(frame);

                while let Some(seg) = vad.pop_segment() {
                    if let Ok(t) = stt.transcribe(&seg) {
                        if !t.is_empty() { println!("\r\x1B[2K{t}"); }
                        else { print!("\r\x1B[2K"); std::io::stdout().flush().ok(); }
                    }
                    pbuf.clear(); since_partial = 0;
                }

                if vad.is_speech() {
                    pbuf.extend_from_slice(frame);
                    since_partial += n;
                    if since_partial >= partial_every && pbuf.len() > sample_rate as usize / 2 {
                        if let Ok(t) = stt.transcribe(&pbuf) {
                            if !t.is_empty() { print!("\r\x1B[2K{t}"); std::io::stdout().flush().ok(); }
                        }
                        since_partial = 0;
                    }
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("STT thread: {e}")))?;

        info!("STT active. Ctrl+C to stop.");
        self.wait_for_shutdown();
        Ok(())
    }

    /// Full pipeline: mic → Silero VAD → GigaAM STT → Qwen translation → console.
    ///
    /// Display: two lines, [RU] and [EN], updating in real-time.
    /// Each line truncated to terminal width to prevent wrapping artifacts.
    fn run_full(&self) -> Result<()> {
        info!("Starting full pipeline");
        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;

        let (producer, mut consumer) = RingBuffer::<f32>::new(sample_rate as usize * 8);
        let input_device = device::default_input_device()?;
        let _input_stream = capture::start_capture_converted(&input_device, sample_rate, producer)?;

        let (stt_tx, stt_rx) = bounded::<SttEvent>(16);
        let display = Arc::new(Mutex::new(TwoLineDisplay::new()));

        // --- STT thread ---
        let shutdown_stt = self.shutdown.clone();
        let stt_cfg = self.config.stt.clone();
        let vad_cfg = self.config.vad.clone();
        let src_tag = self.config.languages.source.to_uppercase();
        let d1 = display.clone();

        let _stt = thread::Builder::new().name("stt".into()).spawn(move || {
            let stt = match GigaAmStt::new(&stt_cfg) {
                Ok(s) => s, Err(e) => { error!("STT: {e}"); return; }
            };
            let vad = match VadDetector::new(&vad_cfg, sample_rate) {
                Ok(v) => v, Err(e) => { error!("VAD: {e}"); return; }
            };
            info!("STT + Silero VAD ready");

            let partial_every = sample_rate as usize;
            let chunk = 512usize;
            let mut tmp = vec![0.0f32; chunk];
            let mut pbuf: Vec<f32> = Vec::new();
            let mut since_partial = 0usize;
            let mut last_text = String::new();

            while !shutdown_stt.is_shutdown() {
                let avail = consumer.slots();
                if avail < chunk { thread::sleep(Duration::from_millis(5)); continue; }
                let n = avail.min(chunk);
                for i in 0..n { tmp[i] = consumer.pop().unwrap_or(0.0); }
                let frame = &tmp[..n];
                vad.accept_waveform(frame);

                // Completed segments → final
                while let Some(seg) = vad.pop_segment() {
                    if let Ok(text) = stt.transcribe(&seg) {
                        if !text.is_empty() {
                            d1.lock().unwrap().update_ru(&format!("[{src_tag}] {text}"));
                            let _ = stt_tx.send(SttEvent::Final(text));
                        }
                    }
                    pbuf.clear();
                    since_partial = 0;
                    last_text.clear();
                }

                // Partials while speaking
                if vad.is_speech() {
                    pbuf.extend_from_slice(frame);
                    since_partial += n;
                    if since_partial >= partial_every && pbuf.len() > sample_rate as usize / 2 {
                        if let Ok(text) = stt.transcribe(&pbuf) {
                            if !text.is_empty() {
                                d1.lock().unwrap().update_ru(&format!("[{src_tag}] {text}"));
                                if text != last_text {
                                    last_text = text.clone();
                                    let _ = stt_tx.try_send(SttEvent::Partial(text));
                                }
                            }
                        }
                        since_partial = 0;
                    }
                }
            }
            vad.flush();
            while let Some(seg) = vad.pop_segment() {
                if let Ok(text) = stt.transcribe(&seg) {
                    if !text.is_empty() { let _ = stt_tx.send(SttEvent::Final(text)); }
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("STT thread: {e}")))?;

        // --- Translation thread ---
        let shutdown_tr = self.shutdown.clone();
        let tr_cfg = self.config.translation.clone();
        let src_lang = self.config.languages.source.clone();
        let tgt_lang = self.config.languages.target.clone();
        let d2 = display.clone();

        let _tr = thread::Builder::new().name("translation".into()).spawn(move || {
            let engine = match TranslationEngine::new(&tr_cfg) {
                Ok(e) => e, Err(e) => { error!("Translation: {e}"); return; }
            };
            let mut ctx = match engine.make_context() {
                Ok(c) => c, Err(e) => { error!("Context: {e}"); return; }
            };
            info!("Translation engine ready");
            let tag = tgt_lang.to_uppercase();

            while !shutdown_tr.is_shutdown() {
                let ev = match stt_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(e) => e,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                // Drain to latest
                let mut latest = ev;
                while let Ok(newer) = stt_rx.try_recv() { latest = newer; }

                match latest {
                    SttEvent::Partial(text) if text.len() > 2 => {
                        if let Ok(tr) = engine.translate_with(&mut ctx, &text, &src_lang, &tgt_lang) {
                            if !tr.is_empty() {
                                d2.lock().unwrap().update_en(&format!("[{tag}] {tr}"));
                            }
                        }
                    }
                    SttEvent::Final(text) if text.len() > 2 => {
                        debug!("Translating final: '{text}'");
                        match engine.translate_with(&mut ctx, &text, &src_lang, &tgt_lang) {
                            Ok(tr) if !tr.is_empty() => {
                                d2.lock().unwrap().commit(&format!("[{tag}] {tr}"));
                            }
                            _ => { d2.lock().unwrap().commit_ru_only(); }
                        }
                    }
                    _ => {}
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("Translation thread: {e}")))?;

        info!("Pipeline active. Ctrl+C to stop.");
        self.wait_for_shutdown();
        info!("Pipeline stopped.");
        Ok(())
    }

    fn wait_for_shutdown(&self) {
        let shutdown = self.shutdown.clone();
        let _ = ctrlc::set_handler(move || {
            info!("Shutting down...");
            shutdown.shutdown();
        });
        while !self.shutdown.is_shutdown() {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

// ---- Types ----

enum SttEvent {
    Partial(String),
    Final(String),
}

// ---- Two-line display ----
//
// Always occupies exactly 2 terminal lines when active:
//   Line 1: [RU] ...
//   Line 2: [EN] ...  (may be blank)
//
// Cursor is ALWAYS at line 2 after any render.
// On commit, both lines are printed with println (scroll down).
// Each line is truncated to terminal width to prevent wrapping.

struct TwoLineDisplay {
    ru: String,
    en: String,
    active: bool, // true after first render (we occupy 2 lines)
    term_width: usize,
    ui_mode: bool, // emit clean machine-readable lines instead of ANSI (for GUI)
}

impl TwoLineDisplay {
    fn new() -> Self {
        TwoLineDisplay {
            ru: String::new(),
            en: String::new(),
            active: false,
            term_width: get_terminal_width(),
            ui_mode: std::env::var("VOICE_UI").is_ok(),
        }
    }

    fn update_ru(&mut self, text: &str) {
        self.ru = text.to_string();
        if self.ui_mode {
            emit_ui("RU", strip_tag(text), "");
        } else {
            self.render();
        }
    }

    fn update_en(&mut self, text: &str) {
        self.en = text.to_string();
        if self.ui_mode {
            emit_ui("EN", strip_tag(text), "");
        } else {
            self.render();
        }
    }

    /// Commit both lines with println and reset for next utterance.
    fn commit(&mut self, en_text: &str) {
        self.en = en_text.to_string();
        if self.ui_mode {
            emit_ui("FINAL", strip_tag(&self.ru), strip_tag(en_text));
            self.ru.clear();
            self.en.clear();
            return;
        }
        let mut out = std::io::stdout();
        if self.active {
            // Go up to line 1, clear it
            write!(out, "\x1B[1A\r\x1B[2K").ok();
        }
        // Print committed lines
        writeln!(out, "{}", self.ru).ok();
        writeln!(out, "{}", self.en).ok();
        out.flush().ok();
        self.ru.clear();
        self.en.clear();
        self.active = false;
    }

    /// Commit RU only (no translation).
    fn commit_ru_only(&mut self) {
        if self.ui_mode {
            emit_ui("FINAL", strip_tag(&self.ru), "");
            self.ru.clear();
            self.en.clear();
            return;
        }
        let mut out = std::io::stdout();
        if self.active {
            write!(out, "\x1B[1A\r\x1B[2K").ok();
        }
        if !self.ru.is_empty() {
            writeln!(out, "{}", self.ru).ok();
        }
        // Clear line 2 remains
        write!(out, "\r\x1B[2K").ok();
        out.flush().ok();
        self.ru.clear();
        self.en.clear();
        self.active = false;
    }

    /// Redraw both lines in place. Cursor ends at line 2.
    fn render(&mut self) {
        let w = self.term_width;
        let mut out = std::io::stdout();

        if self.active {
            // Move up from line 2 to line 1
            write!(out, "\x1B[1A").ok();
        }

        // Line 1: [RU]
        write!(out, "\r\x1B[2K{}", self.ru).ok();
        // Line 2: [EN] (or blank)
        write!(out, "\n\r\x1B[2K{}", self.en).ok();
        out.flush().ok();

        self.active = true;
        // Cursor is now at end of line 2
    }
}

/// Emit a clean, tab-delimited record for an external UI to parse.
/// Each record is prefixed with US (0x1F) so the GUI can ignore any other
/// stdout noise. Kinds: "RU"/"EN" carry a live partial in `a`; "FINAL"
/// carries the committed `a`=source and `b`=translation.
fn emit_ui(kind: &str, a: &str, b: &str) {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "\u{1f}{kind}\t{a}\t{b}");
    let _ = out.flush();
}

/// Strip a leading "[XX] " language tag (e.g. "[RU] привет" → "привет").
fn strip_tag(s: &str) -> &str {
    if s.starts_with('[') {
        if let Some(i) = s.find("] ") {
            return &s[i + 2..];
        }
    }
    s
}

#[allow(dead_code)]
fn truncate(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }
    s.chars().take(max_chars).collect()
}

fn get_terminal_width() -> usize {
    // Try COLUMNS env var first, then ioctl, fallback to 200
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(w) = cols.parse::<usize>() {
            if w > 40 { return w; }
        }
    }

    #[cfg(unix)]
    {
        use std::mem::zeroed;
        unsafe {
            let mut ws: libc::winsize = zeroed();
            if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }

    200
}
