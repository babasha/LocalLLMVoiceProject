use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::DeviceTrait;
use crossbeam_channel::bounded;
use rtrb::RingBuffer;
use tracing::{error, info};

use crate::audio::{capture, device, playback};
use crate::config::AppConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::pipeline::messages::*;
use crate::pipeline::shutdown::ShutdownSignal;
use crate::stt::gigaam::GigaAmStt;
use crate::translation::engine::TranslationEngine;

/// Pipeline mode determines which stages are active.
#[derive(Debug, Clone, Copy)]
pub enum PipelineMode {
    /// Audio passthrough: mic → speaker (Phase 1 testing)
    Passthrough,
    /// VAD + STT only: mic → text in console (Phase 2 testing)
    SttOnly,
    /// Full pipeline: mic → VAD → STT → Translation → TTS → speaker
    Full,
}

/// Orchestrates the full translation pipeline with 6 threads.
pub struct Orchestrator {
    config: AppConfig,
    mode: PipelineMode,
    shutdown: ShutdownSignal,
}

impl Orchestrator {
    pub fn new(config: AppConfig, mode: PipelineMode) -> Self {
        let shutdown = ShutdownSignal::new();
        Orchestrator {
            config,
            mode,
            shutdown,
        }
    }

    /// Run the pipeline. Blocks until Ctrl+C or error.
    pub fn run(&self) -> Result<()> {
        match self.mode {
            PipelineMode::Passthrough => self.run_passthrough(),
            PipelineMode::SttOnly => self.run_stt_only(),
            PipelineMode::Full => self.run_full(),
        }
    }

    /// Phase 1: Simple mic → speaker passthrough via rtrb ring buffer.
    fn run_passthrough(&self) -> Result<()> {
        info!("Starting passthrough mode (mic → speaker)");

        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;
        let capacity = self.config.audio.ring_buffer_capacity;

        // Create ring buffer
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);

        // Get devices
        let input_device = device::default_input_device()?;
        let output_device = device::default_output_device()?;

        info!(
            "Input:  {}",
            input_device.name().unwrap_or_default()
        );
        info!(
            "Output: {}",
            output_device.name().unwrap_or_default()
        );

        // Start streams
        let _input_stream =
            capture::start_capture(&input_device, sample_rate, channels, producer)?;
        let _output_stream =
            playback::start_playback(&output_device, sample_rate, channels, consumer)?;

        info!("Passthrough active. Press Ctrl+C to stop.");

        // Block until shutdown
        self.wait_for_shutdown();

        info!("Passthrough stopped.");
        Ok(())
    }

    /// Phase 2: Streaming STT — partial results update in real-time like voice assistants.
    fn run_stt_only(&self) -> Result<()> {
        info!("Starting streaming STT mode (mic → text)");

        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;
        // Large ring buffer: 8 seconds to absorb latency during transcription
        let capacity = sample_rate as usize * 8;

        let (producer, mut consumer) = RingBuffer::<f32>::new(capacity);

        let input_device = device::default_input_device()?;
        let _input_stream =
            capture::start_capture(&input_device, sample_rate, channels, producer)?;

        let shutdown_stt = self.shutdown.clone();
        let stt_config = self.config.stt.clone();
        let vad_threshold = self.config.vad.threshold;
        let silence_duration_ms = self.config.vad.silence_duration_ms;

        let _stt_handle = thread::Builder::new()
            .name("stt".into())
            .spawn(move || {
                let stt = match GigaAmStt::new(&stt_config) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Failed to initialize GigaAM STT: {}", e);
                        return;
                    }
                };
                info!("STT engine ready — streaming mode");
                println!("--- говорите ---");

                // Energy threshold: same formula as VAD detector
                let energy_threshold = vad_threshold * 0.05;
                // How many samples of new speech before we run a partial transcription
                let partial_every = sample_rate as usize; // ~1 second
                // Adaptive silence: base from config, extended for longer utterances
                let silence_base = (sample_rate as f32 * silence_duration_ms as f32 / 1000.0) as usize;
                let silence_long = sample_rate as usize * 4; // 4s for long utterances
                let long_utterance_threshold = sample_rate as usize * 3; // 3s of speech = "long"

                let chunk = 480usize; // 30ms frame
                let mut tmp = vec![0.0f32; chunk];

                // Audio accumulated during current utterance
                let mut utterance: Vec<f32> = Vec::new();
                let mut samples_since_partial = 0usize;
                let mut silent_samples = 0usize;
                let mut in_speech = false;

                while !shutdown_stt.is_shutdown() {
                    let available = consumer.slots();
                    if available < chunk {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let to_read = available.min(chunk);
                    for i in 0..to_read {
                        tmp[i] = consumer.pop().unwrap_or(0.0);
                    }
                    let frame = &tmp[..to_read];
                    let energy = rms_energy(frame);
                    let is_speech = energy > energy_threshold;

                    if is_speech {
                        if !in_speech {
                            in_speech = true;
                        }
                        silent_samples = 0;
                        utterance.extend_from_slice(frame);
                        samples_since_partial += to_read;

                        // Partial transcription every ~1 second of new speech
                        if samples_since_partial >= partial_every && utterance.len() > sample_rate as usize / 2 {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    print!("\r\x1B[2K{}", text);
                                    std::io::stdout().flush().ok();
                                }
                            }
                            samples_since_partial = 0;
                        }

                        // Force-flush after 15 seconds (for noisy environments)
                        if utterance.len() >= sample_rate as usize * 15 {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    println!("\r\x1B[2K{}", text);
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            in_speech = false;
                        }
                    } else if in_speech {
                        utterance.extend_from_slice(frame);
                        silent_samples += to_read;

                        // Adaptive: long utterances get more patience for pauses
                        let effective_silence = if utterance.len() > long_utterance_threshold {
                            silence_long
                        } else {
                            silence_base
                        };

                        if silent_samples >= effective_silence {
                            // End of utterance — final transcription
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    println!("\r\x1B[2K{}", text);
                                } else {
                                    print!("\r\x1B[2K");
                                    std::io::stdout().flush().ok();
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            silent_samples = 0;
                            in_speech = false;
                        }
                    }
                }
            })
            .map_err(|e| {
                VoiceTranslatorError::Pipeline(format!("Failed to spawn STT thread: {}", e))
            })?;

        info!("Streaming STT active. Press Ctrl+C to stop.");
        self.wait_for_shutdown();

        info!("STT-only mode stopped.");
        Ok(())
    }

    /// Full pipeline: streaming STT → parallel streaming translation → console output.
    fn run_full(&self) -> Result<()> {
        info!("Starting full pipeline: STT + streaming Translation");

        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;
        let capacity = sample_rate as usize * 8;

        let (producer, mut consumer) = RingBuffer::<f32>::new(capacity);
        let input_device = device::default_input_device()?;
        let _input_stream =
            capture::start_capture(&input_device, sample_rate, channels, producer)?;

        // Channel: STT → Translation (partials + finals)
        let (stt_tx, stt_rx) = bounded::<SttEvent>(16);

        // Shared two-line display
        let display = Arc::new(Mutex::new(LiveDisplay::new()));

        // STT thread
        let shutdown_stt = self.shutdown.clone();
        let stt_config = self.config.stt.clone();
        let vad_threshold = self.config.vad.threshold;
        let silence_duration_ms = self.config.vad.silence_duration_ms;
        let source_lang_stt = self.config.languages.source.clone();
        let display_stt = display.clone();

        let _stt_handle = thread::Builder::new()
            .name("stt".into())
            .spawn(move || {
                let stt = match GigaAmStt::new(&stt_config) {
                    Ok(s) => s,
                    Err(e) => { error!("STT init failed: {}", e); return; }
                };
                info!("STT engine ready");

                let energy_threshold = vad_threshold * 0.05;
                let partial_every = sample_rate as usize;
                let silence_base = (sample_rate as f32 * silence_duration_ms as f32 / 1000.0) as usize;
                let silence_long = sample_rate as usize * 4;
                let long_utterance_threshold = sample_rate as usize * 3;
                let force_flush_samples = sample_rate as usize * 15;

                let chunk = 480usize;
                let mut tmp = vec![0.0f32; chunk];
                let mut utterance: Vec<f32> = Vec::new();
                let mut samples_since_partial = 0usize;
                let mut silent_samples = 0usize;
                let mut in_speech = false;
                let mut last_partial_text = String::new();

                while !shutdown_stt.is_shutdown() {
                    let available = consumer.slots();
                    if available < chunk {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let to_read = available.min(chunk);
                    for i in 0..to_read { tmp[i] = consumer.pop().unwrap_or(0.0); }
                    let frame = &tmp[..to_read];
                    let is_speech = rms_energy(frame) > energy_threshold;

                    if is_speech {
                        if !in_speech { in_speech = true; }
                        silent_samples = 0;
                        utterance.extend_from_slice(frame);
                        samples_since_partial += to_read;

                        // Partial transcription every ~1 second
                        if samples_since_partial >= partial_every && utterance.len() > sample_rate as usize / 2 {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    let ru = format!("[{}] {}", source_lang_stt.to_uppercase(), text);
                                    display_stt.lock().unwrap().update_ru(&ru);
                                    // Only send to translation if text actually changed
                                    if text != last_partial_text {
                                        last_partial_text = text.clone();
                                        let _ = stt_tx.try_send(SttEvent::Partial(text));
                                    }
                                }
                            }
                            samples_since_partial = 0;
                        }

                        // Force-flush if speech is too long
                        if utterance.len() >= force_flush_samples {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    let ru = format!("[{}] {}", source_lang_stt.to_uppercase(), text);
                                    display_stt.lock().unwrap().finalize_ru(&ru);
                                    let _ = stt_tx.send(SttEvent::Final(text));
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            last_partial_text.clear();
                            in_speech = false;
                        }
                    } else if in_speech {
                        utterance.extend_from_slice(frame);
                        silent_samples += to_read;

                        let effective_silence = if utterance.len() > long_utterance_threshold {
                            silence_long
                        } else {
                            silence_base
                        };

                        if silent_samples >= effective_silence {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    let ru = format!("[{}] {}", source_lang_stt.to_uppercase(), text);
                                    display_stt.lock().unwrap().finalize_ru(&ru);
                                    let _ = stt_tx.send(SttEvent::Final(text));
                                } else {
                                    display_stt.lock().unwrap().clear_partial();
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            last_partial_text.clear();
                            silent_samples = 0;
                            in_speech = false;
                        }
                    }
                }
            })
            .map_err(|e| VoiceTranslatorError::Pipeline(format!("Failed to spawn STT thread: {e}")))?;

        // Translation thread — processes both partials and finals
        let shutdown_trans = self.shutdown.clone();
        let translation_config = self.config.translation.clone();
        let source_lang = self.config.languages.source.clone();
        let target_lang = self.config.languages.target.clone();
        let display_trans = display.clone();

        let _translation_handle = thread::Builder::new()
            .name("translation".into())
            .spawn(move || {
                let engine = match TranslationEngine::new(&translation_config) {
                    Ok(e) => e,
                    Err(e) => { error!("Translation engine init failed: {}", e); return; }
                };

                let mut ctx = match engine.make_context() {
                    Ok(c) => c,
                    Err(e) => { error!("Failed to create translation context: {}", e); return; }
                };

                info!("Translation engine ready (streaming mode)");
                let tag = target_lang.to_uppercase();

                while !shutdown_trans.is_shutdown() {
                    // Wait for next event
                    let event = match stt_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(e) => e,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    };

                    // Drain channel to get the latest event (skip stale partials)
                    let mut latest = event;
                    while let Ok(newer) = stt_rx.try_recv() {
                        latest = newer;
                    }

                    match latest {
                        SttEvent::Partial(text) if text.len() > 2 => {
                            match engine.translate_with(&mut ctx, &text, &source_lang, &target_lang) {
                                Ok(translated) if !translated.is_empty() => {
                                    let en = format!("[{}] {}", tag, translated);
                                    display_trans.lock().unwrap().update_en(&en);
                                }
                                _ => {}
                            }
                        }
                        SttEvent::Final(text) if text.len() > 2 => {
                            info!("Translating final: '{}'", text);
                            match engine.translate_with(&mut ctx, &text, &source_lang, &target_lang) {
                                Ok(translated) if !translated.is_empty() => {
                                    let en = format!("[{}] {}", tag, translated);
                                    display_trans.lock().unwrap().finalize_en(&en);
                                }
                                Ok(_) => {
                                    display_trans.lock().unwrap().finalize_empty();
                                }
                                Err(e) => {
                                    error!("Translation error: {}", e);
                                    display_trans.lock().unwrap().finalize_empty();
                                }
                            }
                        }
                        _ => {} // skip very short noise artifacts
                    }
                }
            })
            .map_err(|e| {
                VoiceTranslatorError::Pipeline(format!("Failed to spawn translation thread: {e}"))
            })?;

        info!("Full pipeline active (streaming STT + Translation). Press Ctrl+C to stop.");
        self.wait_for_shutdown();

        info!("Full pipeline stopped.");
        Ok(())
    }

    fn wait_for_shutdown(&self) {
        // Wait for Ctrl+C
        let shutdown = self.shutdown.clone();
        let _ = ctrlc::set_handler(move || {
            info!("Ctrl+C received, shutting down...");
            shutdown.shutdown();
        });

        while !self.shutdown.is_shutdown() {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Event from STT thread to Translation thread.
enum SttEvent {
    /// Intermediate transcription — translate and show as updating [EN] line.
    Partial(String),
    /// Final transcription (utterance complete) — translate and commit both lines.
    Final(String),
}

/// Manages two-line terminal display: [RU] and [EN] updating in real-time.
struct LiveDisplay {
    ru_text: String,
    en_text: String,
    /// How many lines we currently occupy (1 = RU only, 2 = RU + EN)
    lines: u32,
}

impl LiveDisplay {
    fn new() -> Self {
        LiveDisplay {
            ru_text: String::new(),
            en_text: String::new(),
            lines: 0,
        }
    }

    /// Update [RU] partial text (overwrites in place).
    fn update_ru(&mut self, text: &str) {
        self.ru_text = text.to_string();
        self.render_live();
    }

    /// Update [EN] partial text (overwrites in place).
    fn update_en(&mut self, text: &str) {
        self.en_text = text.to_string();
        self.render_live();
    }

    /// Finalize [RU] line (utterance complete, waiting for translation).
    fn finalize_ru(&mut self, text: &str) {
        self.ru_text = text.to_string();
        self.render_live();
    }

    /// Finalize [EN] line and commit both lines (move to next pair).
    fn finalize_en(&mut self, text: &str) {
        self.en_text = text.to_string();
        self.render_commit();
    }

    /// Commit without EN text (noise/empty result).
    fn finalize_empty(&mut self) {
        self.render_commit();
    }

    /// Clear partial display (empty transcription).
    fn clear_partial(&mut self) {
        self.move_to_top();
        let mut stdout = std::io::stdout();
        write!(stdout, "\r\x1B[2K").ok();
        if self.lines > 1 {
            write!(stdout, "\n\r\x1B[2K\x1B[1A").ok();
        }
        stdout.flush().ok();
        self.ru_text.clear();
        self.en_text.clear();
        self.lines = 0;
    }

    /// Redraw both lines in place (no newline — stays on same position).
    fn render_live(&mut self) {
        self.move_to_top();
        let mut stdout = std::io::stdout();

        // Write RU line
        write!(stdout, "\r\x1B[2K{}", self.ru_text).ok();

        if !self.en_text.is_empty() {
            // Write EN line below
            write!(stdout, "\n\r\x1B[2K{}", self.en_text).ok();
            self.lines = 2;
        } else {
            self.lines = 1;
        }
        stdout.flush().ok();
    }

    /// Commit both lines (println) and reset for next utterance.
    fn render_commit(&mut self) {
        self.move_to_top();
        let mut stdout = std::io::stdout();

        // Print RU (committed)
        write!(stdout, "\r\x1B[2K{}", self.ru_text).ok();

        if !self.en_text.is_empty() {
            // Print EN below
            write!(stdout, "\n\r\x1B[2K{}", self.en_text).ok();
        }

        // Newline to commit — next utterance starts on fresh line
        writeln!(stdout).ok();
        stdout.flush().ok();

        self.ru_text.clear();
        self.en_text.clear();
        self.lines = 0;
    }

    /// Move cursor back to the top of our display area.
    fn move_to_top(&self) {
        if self.lines > 1 {
            let mut stdout = std::io::stdout();
            // Move up from EN line to RU line
            for _ in 1..self.lines {
                write!(stdout, "\x1B[1A").ok();
            }
        }
    }
}

fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}
