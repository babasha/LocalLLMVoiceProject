use std::thread;
use std::time::Duration;

use cpal::traits::DeviceTrait;
use crossbeam_channel::{bounded, Receiver, Sender};
use rtrb::RingBuffer;
use tracing::{error, info};

use crate::audio::{capture, device, playback};
use crate::config::AppConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::pipeline::messages::*;
use crate::pipeline::shutdown::ShutdownSignal;
use crate::vad::detector::VadDetector;

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

    /// Phase 2: VAD + STT pipeline (prints transcribed text).
    fn run_stt_only(&self) -> Result<()> {
        info!("Starting STT-only mode (mic → text)");

        let sample_rate = self.config.audio.sample_rate;
        let channels = self.config.audio.channels;
        let capacity = self.config.audio.ring_buffer_capacity;

        // Ring buffer: capture → VAD reader
        let (producer, mut consumer) = RingBuffer::<f32>::new(capacity);

        // Channel: VAD → STT
        let (vad_tx, _vad_rx): (Sender<VadToStt>, Receiver<VadToStt>) = bounded(4);

        // Get input device
        let input_device = device::default_input_device()?;
        let _input_stream =
            capture::start_capture(&input_device, sample_rate, channels, producer)?;

        // VAD thread
        let shutdown = self.shutdown.clone();
        let vad_config = self.config.vad.clone();
        let _vad_handle = thread::Builder::new()
            .name("vad".into())
            .spawn(move || {
                let mut detector = match VadDetector::new(&vad_config, sample_rate) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Failed to create VAD: {}", e);
                        return;
                    }
                };

                let mut read_buf = vec![0.0f32; 480]; // 30ms at 16kHz

                while !shutdown.is_shutdown() {
                    // Read available samples from ring buffer
                    let available = consumer.slots();
                    if available == 0 {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }

                    let to_read = available.min(read_buf.len());
                    for i in 0..to_read {
                        read_buf[i] = consumer.pop().unwrap_or(0.0);
                    }

                    let segments = detector.process(&read_buf[..to_read]);
                    for segment in segments {
                        info!(
                            "Speech segment: {:.2}s ({} samples)",
                            segment.duration_secs,
                            segment.audio.len()
                        );
                        let msg = VadToStt { segment };
                        if vad_tx.send(msg).is_err() {
                            break;
                        }
                    }
                }

                // Flush remaining
                if let Some(segment) = detector.flush() {
                    let _ = vad_tx.send(VadToStt { segment });
                }
            })
            .map_err(|e| VoiceTranslatorError::Pipeline(format!("Failed to spawn VAD thread: {}", e)))?;

        info!("STT-only mode active. Speak into your microphone. Press Ctrl+C to stop.");
        self.wait_for_shutdown();

        info!("STT-only mode stopped.");
        Ok(())
    }

    /// Full pipeline (Phase 4+).
    fn run_full(&self) -> Result<()> {
        info!("Full pipeline mode not yet implemented. Use --mode passthrough or --mode stt");
        Err(VoiceTranslatorError::Pipeline(
            "Full pipeline not yet implemented".into(),
        ))
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
