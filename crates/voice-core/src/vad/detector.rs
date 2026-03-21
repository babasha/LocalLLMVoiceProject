use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector};
use tracing::info;

use crate::config::VadConfig;
use crate::error::{Result, VoiceTranslatorError};

/// Voice Activity Detector using Silero VAD v5 via sherpa-onnx.
pub struct VadDetector {
    vad: VoiceActivityDetector,
    sample_rate: u32,
}

impl VadDetector {
    pub fn new(config: &VadConfig, sample_rate: u32) -> Result<Self> {
        let vad_config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(config.model_path.clone()),
                threshold: config.threshold,
                min_silence_duration: config.silence_duration_ms as f32 / 1000.0,
                min_speech_duration: config.min_speech_duration_ms as f32 / 1000.0,
                window_size: 512,
                max_speech_duration: config.max_speech_duration_ms as f32 / 1000.0,
            },
            sample_rate: sample_rate as i32,
            num_threads: 2,
            ..Default::default()
        };

        let vad = VoiceActivityDetector::create(&vad_config, 60.0).ok_or_else(|| {
            VoiceTranslatorError::Pipeline("Failed to create Silero VAD".into())
        })?;

        info!(
            "Silero VAD initialized: threshold={}, silence={}ms, max_speech={}ms",
            config.threshold, config.silence_duration_ms, config.max_speech_duration_ms
        );

        Ok(VadDetector { vad, sample_rate })
    }

    /// Feed audio samples to the VAD.
    pub fn accept_waveform(&self, samples: &[f32]) {
        self.vad.accept_waveform(samples);
    }

    /// Returns true if speech is currently being detected.
    pub fn is_speech(&self) -> bool {
        self.vad.detected()
    }

    /// Returns true if there are completed speech segments available.
    pub fn has_segment(&self) -> bool {
        !self.vad.is_empty()
    }

    /// Get the next completed speech segment's audio and remove it from the queue.
    pub fn pop_segment(&self) -> Option<Vec<f32>> {
        if self.vad.is_empty() {
            return None;
        }
        let segment = self.vad.front()?;
        let audio = segment.samples().to_vec();
        self.vad.pop();
        Some(audio)
    }

    /// Flush any trailing speech on shutdown.
    pub fn flush(&self) {
        self.vad.flush();
    }

    /// Reset VAD state.
    pub fn reset(&self) {
        self.vad.reset();
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
