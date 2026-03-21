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
                // How many silent samples before we finalise an utterance
                let silence_limit = (sample_rate as f32 * 0.6) as usize; // 600ms

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

                        // Force-flush after 5 seconds (for noisy environments)
                        if utterance.len() >= sample_rate as usize * 5 {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    println!("\r\x1B[2K{}", text);
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            in_speech = false;
                        }

                        // Sliding window: keep last 8 seconds max
                        let max_samples = sample_rate as usize * 8;
                        if utterance.len() > max_samples {
                            utterance.drain(..utterance.len() - max_samples);
                        }
                    } else if in_speech {
                        utterance.extend_from_slice(frame);
                        silent_samples += to_read;

                        if silent_samples >= silence_limit {
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

    /// Full pipeline: streaming STT → Qwen translation → console output.
    fn run_full(&self) -> Result<()> {
        info!("Starting full pipeline: STT + Translation");

        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;
        let capacity = sample_rate as usize * 8;

        let (producer, mut consumer) = RingBuffer::<f32>::new(capacity);
        let input_device = device::default_input_device()?;
        let _input_stream =
            capture::start_capture(&input_device, sample_rate, channels, producer)?;

        // Channel: STT final utterances → Translation
        let (stt_tx, stt_rx) = bounded::<String>(8);

        // Shared stdout lock — prevents STT partials from overwriting translation output
        let stdout_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));

        // STT thread
        let shutdown_stt = self.shutdown.clone();
        let stt_config = self.config.stt.clone();
        let vad_threshold = self.config.vad.threshold;
        let source_lang_stt = self.config.languages.source.clone();
        let lock_stt = stdout_lock.clone();

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
                let silence_limit = (sample_rate as f32 * 0.6) as usize;
                let chunk = 480usize;
                let mut tmp = vec![0.0f32; chunk];
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
                    for i in 0..to_read { tmp[i] = consumer.pop().unwrap_or(0.0); }
                    let frame = &tmp[..to_read];
                    let is_speech = rms_energy(frame) > energy_threshold;

                    // Force-flush after 5 seconds of speech (handles noisy environments)
                    let force_flush_samples = sample_rate as usize * 5;

                    if is_speech {
                        if !in_speech { in_speech = true; }
                        silent_samples = 0;
                        utterance.extend_from_slice(frame);
                        samples_since_partial += to_read;

                        if samples_since_partial >= partial_every && utterance.len() > sample_rate as usize / 2 {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    let _g = lock_stt.lock().unwrap();
                                    print!("\r\x1B[2K[{}] {}", source_lang_stt.to_uppercase(), text);
                                    std::io::stdout().flush().ok();
                                }
                            }
                            samples_since_partial = 0;
                        }

                        // Force-flush if speech is too long (no silence detected)
                        if utterance.len() >= force_flush_samples {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    {
                                        let _g = lock_stt.lock().unwrap();
                                        println!("\r\x1B[2K[{}] {}", source_lang_stt.to_uppercase(), text);
                                    }
                                    let _ = stt_tx.send(text);
                                }
                            }
                            utterance.clear();
                            samples_since_partial = 0;
                            in_speech = false;
                        }
                    } else if in_speech {
                        utterance.extend_from_slice(frame);
                        silent_samples += to_read;

                        if silent_samples >= silence_limit {
                            if let Ok(text) = stt.transcribe(&utterance) {
                                if !text.is_empty() {
                                    {
                                        let _g = lock_stt.lock().unwrap();
                                        println!("\r\x1B[2K[{}] {}", source_lang_stt.to_uppercase(), text);
                                    }
                                    let _ = stt_tx.send(text);
                                } else {
                                    let _g = lock_stt.lock().unwrap();
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
            .map_err(|e| VoiceTranslatorError::Pipeline(format!("Failed to spawn STT thread: {e}")))?;

        // Translation thread — loads Qwen3.5-4B, creates context ONCE, reuses it
        let shutdown_trans = self.shutdown.clone();
        let translation_config = self.config.translation.clone();
        let source_lang = self.config.languages.source.clone();
        let target_lang = self.config.languages.target.clone();
        let lock_trans = stdout_lock.clone();

        let _translation_handle = thread::Builder::new()
            .name("translation".into())
            .spawn(move || {
                let engine = match TranslationEngine::new(&translation_config) {
                    Ok(e) => e,
                    Err(e) => { error!("Translation engine init failed: {}", e); return; }
                };

                // Create context once — no re-allocation per call
                let mut ctx = match engine.make_context() {
                    Ok(c) => c,
                    Err(e) => { error!("Failed to create translation context: {}", e); return; }
                };

                info!("Translation engine ready (persistent context)");

                while !shutdown_trans.is_shutdown() {
                    match stt_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(text) if text.len() > 2 => {
                            info!("Translating: '{}'", text);
                            match engine.translate_with(&mut ctx, &text, &source_lang, &target_lang) {
                                Ok(translated) if !translated.is_empty() => {
                                    let _g = lock_trans.lock().unwrap();
                                    println!("\r\x1B[2K[{}] {}", target_lang.to_uppercase(), translated);
                                }
                                Ok(_) => {}
                                Err(e) => error!("Translation error: {}", e),
                            }
                        }
                        Ok(_) => {} // skip very short noise artifacts
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|e| {
                VoiceTranslatorError::Pipeline(format!("Failed to spawn translation thread: {e}"))
            })?;

        info!("Full pipeline active (STT → Translation). Press Ctrl+C to stop.");
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

fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}
