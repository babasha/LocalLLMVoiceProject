use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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

            let perf = std::env::var("VOICE_PERF").is_ok();
            // Chunk-based streaming ASR with LocalAgreement-2 (Whisper-Streaming
            // style): re-transcribe the growing utterance buffer ~every 0.4s and
            // commit only the RU prefix that agreed between the two most recent
            // transcriptions. Stable RU words are streamed to translation before
            // the speaker pauses; VAD still bounds/segments the utterance.
            // Faster cadence ⇒ words commit sooner (LA-2 latency ≈ 2 intervals);
            // STT RTF ≈ 0.02 so re-transcribing the buffer this often is cheap,
            // and the translation thread self-throttles by draining to latest.
            let partial_every = sample_rate as usize * 2 / 5;
            let chunk = 512usize;
            let mut tmp = vec![0.0f32; chunk];
            let mut pbuf: Vec<f32> = Vec::new();
            let mut since_partial = 0usize;
            let mut prev_text = String::new(); // previous transcription (for agreement)
            let mut committed = 0usize; // RU words already emitted as stable

            while !shutdown_stt.is_shutdown() {
                let avail = consumer.slots();
                if avail < chunk { thread::sleep(Duration::from_millis(5)); continue; }
                let n = avail.min(chunk);
                for i in 0..n { tmp[i] = consumer.pop().unwrap_or(0.0); }
                let frame = &tmp[..n];
                vad.accept_waveform(frame);

                // Completed segments → final transcription, then reset LA state.
                while let Some(seg) = vad.pop_segment() {
                    let t_stt = perf.then(Instant::now);
                    let tr = stt.transcribe(&seg);
                    if let Some(t) = t_stt {
                        let audio_s = seg.len() as f64 / sample_rate as f64;
                        let stt_ms = t.elapsed().as_secs_f64() * 1000.0;
                        let rtf = if audio_s > 0.0 { (stt_ms / 1000.0) / audio_s } else { 0.0 };
                        eprintln!("[PERF] STT: {stt_ms:.0}ms for {audio_s:.2}s audio (RTF {rtf:.2})");
                    }
                    if let Ok(text) = tr {
                        if !text.is_empty() {
                            d1.lock().unwrap().update_ru(&format!("[{src_tag}] {text}"));
                            let _ = stt_tx.send(SttEvent::Final(text));
                        }
                    }
                    pbuf.clear();
                    since_partial = 0;
                    prev_text.clear();
                    committed = 0;
                }

                // While speaking: grow the buffer and run LocalAgreement-2.
                if vad.is_speech() {
                    pbuf.extend_from_slice(frame);
                    since_partial += n;
                    if since_partial >= partial_every && pbuf.len() > sample_rate as usize / 2 {
                        if let Ok(cur) = stt.transcribe(&pbuf) {
                            if !cur.is_empty() {
                                // Show the full live transcription on screen...
                                d1.lock().unwrap().update_ru(&format!("[{src_tag}] {cur}"));
                                // ...but only stream the agreed (stable) prefix.
                                let agreed = common_word_prefix(&prev_text, &cur);
                                if agreed > committed {
                                    committed = agreed;
                                    let stable: String = cur
                                        .split_whitespace()
                                        .take(agreed)
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    let _ = stt_tx.try_send(SttEvent::Partial(stable));
                                }
                                prev_text = cur;
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
        let tts_cfg = self.config.tts.clone();
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

            // Optional: speak the translation aloud (Piper streaming, or SAPI).
            let speaker = if tts_cfg.speak {
                match crate::tts::create_speaker(&tts_cfg) {
                    Ok(s) => Some(s),
                    Err(e) => { error!("TTS disabled: {e}"); None }
                }
            } else {
                None
            };

            // Incremental ("simultaneous") streaming state, per segment:
            //   spoken — the exact target-language text we have already
            //   committed (voiced + shown). It is fed back to the model as a
            //   forced prefix so each new partial *continues* it instead of
            //   re-translating from scratch; the model therefore can never
            //   contradict words the listener already heard. We voice the newly
            //   generated tail except the last HOLD words, which may still
            //   change as more of the sentence arrives. The final flushes
            //   whatever remains, then state resets for the next segment.
            const HOLD: usize = 2;
            let mut spoken = String::new();

            while !shutdown_tr.is_shutdown() {
                let ev = match stt_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(e) => e,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                // Drain the channel; keep the newest partial and the newest final.
                let mut events = vec![ev];
                while let Ok(newer) = stt_rx.try_recv() { events.push(newer); }
                let mut latest_partial: Option<String> = None;
                let mut latest_final: Option<String> = None;
                for e in events {
                    match e {
                        SttEvent::Partial(t) if t.len() > 2 => latest_partial = Some(t),
                        SttEvent::Final(t) if t.len() > 2 => latest_final = Some(t),
                        _ => {}
                    }
                }

                // A final wins: continue the committed prefix to the end of the
                // segment, voice everything still unspoken, then reset.
                if let Some(text) = latest_final {
                    debug!("Translating final: '{text}'");
                    match engine.translate_prefixed(&mut ctx, &text, &spoken, &src_lang, &tgt_lang) {
                        Ok(tr) if !tr.is_empty() => {
                            d2.lock().unwrap().commit(&format!("[{tag}] {tr}"));
                            let already = spoken.split_whitespace().count();
                            let words: Vec<&str> = tr.split_whitespace().collect();
                            if words.len() > already {
                                if let Some(s) = &speaker {
                                    s.speak(&words[already..].join(" "));
                                }
                            }
                        }
                        _ => { d2.lock().unwrap().commit_ru_only(); }
                    }
                    spoken.clear();
                    continue;
                }

                // Otherwise continue the committed prefix from the latest partial
                // and commit/voice the freshly settled words (all but the last
                // HOLD, which may still change as more speech arrives).
                if let Some(text) = latest_partial {
                    if let Ok(tr) = engine.translate_prefixed(&mut ctx, &text, &spoken, &src_lang, &tgt_lang) {
                        if !tr.is_empty() {
                            d2.lock().unwrap().update_en(&format!("[{tag}] {tr}"));
                            let already = spoken.split_whitespace().count();
                            let words: Vec<&str> = tr.split_whitespace().collect();
                            let upto = words.len().saturating_sub(HOLD);
                            if upto > already {
                                if let Some(s) = &speaker {
                                    s.speak(&words[already..upto].join(" "));
                                }
                                spoken = words[..upto].join(" ");
                            }
                        }
                    }
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

/// Number of leading whitespace-separated words that `a` and `b` share.
/// Used for streaming "local agreement": the common prefix of two consecutive
/// partial translations is considered stable enough to speak.
fn common_word_prefix(a: &str, b: &str) -> usize {
    a.split_whitespace()
        .zip(b.split_whitespace())
        .take_while(|(x, y)| x == y)
        .count()
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
