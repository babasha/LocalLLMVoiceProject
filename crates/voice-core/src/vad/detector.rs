use tracing::info;

use crate::config::VadConfig;
use crate::error::Result;
use crate::vad::segment::SpeechSegment;

/// Voice Activity Detector using Silero VAD.
/// Accumulates audio samples and emits SpeechSegments when speech ends.
pub struct VadDetector {
    config: VadConfig,
    sample_rate: u32,
    /// Accumulated audio during active speech
    speech_buffer: Vec<f32>,
    /// Whether we are currently in a speech region
    is_speaking: bool,
    /// Number of consecutive silent frames
    silent_frames: u32,
    /// Samples per VAD frame (typically 30ms = 480 samples at 16kHz)
    frame_size: usize,
    /// Max samples before force-flushing (30 seconds)
    max_segment_samples: usize,
}

impl VadDetector {
    pub fn new(config: &VadConfig, sample_rate: u32) -> Result<Self> {
        let frame_size = (sample_rate as usize * 30) / 1000; // 30ms frames

        info!(
            "VAD initialized: threshold={}, silence={}ms, frame_size={}",
            config.threshold, config.silence_duration_ms, frame_size
        );

        let max_segment_samples = sample_rate as usize * 4; // 4s hard limit for near-realtime

        Ok(VadDetector {
            config: config.clone(),
            sample_rate,
            speech_buffer: Vec::with_capacity(sample_rate as usize * 10),
            is_speaking: false,
            silent_frames: 0,
            frame_size,
            max_segment_samples,
        })
    }

    /// Process a chunk of audio samples. Returns completed speech segments.
    /// TODO: Integrate actual Silero VAD model in Phase 2.
    /// For now, uses simple energy-based detection as placeholder.
    pub fn process(&mut self, samples: &[f32]) -> Vec<SpeechSegment> {
        let mut segments = Vec::new();
        let silence_frames_threshold =
            (self.config.silence_duration_ms as f32 / 30.0).ceil() as u32;
        let min_speech_samples =
            (self.config.min_speech_duration_ms as f32 * self.sample_rate as f32 / 1000.0) as usize;

        // Process in frames
        for chunk in samples.chunks(self.frame_size) {
            let energy = rms_energy(chunk);
            // threshold=0.5 in config → effective energy ~0.025 (filters room noise ~0.005)
            let is_speech = energy > self.config.threshold * 0.05;

            if is_speech {
                self.silent_frames = 0;
                if !self.is_speaking {
                    self.is_speaking = true;
                }
                self.speech_buffer.extend_from_slice(chunk);
            } else if self.is_speaking {
                self.silent_frames += 1;
                self.speech_buffer.extend_from_slice(chunk);

                if self.silent_frames >= silence_frames_threshold {
                    if self.speech_buffer.len() >= min_speech_samples {
                        let segment =
                            SpeechSegment::new(self.speech_buffer.drain(..).collect(), self.sample_rate);
                        segments.push(segment);
                    } else {
                        self.speech_buffer.clear();
                    }
                    self.is_speaking = false;
                    self.silent_frames = 0;
                }
            }

            // Hard limit: force flush if segment grew too long
            if self.speech_buffer.len() >= self.max_segment_samples {
                let segment =
                    SpeechSegment::new(self.speech_buffer.drain(..).collect(), self.sample_rate);
                segments.push(segment);
                self.is_speaking = false;
                self.silent_frames = 0;
            }
        }

        segments
    }

    /// Force-flush any remaining speech buffer (e.g., on shutdown).
    pub fn flush(&mut self) -> Option<SpeechSegment> {
        if self.speech_buffer.is_empty() {
            return None;
        }
        let segment = SpeechSegment::new(self.speech_buffer.drain(..).collect(), self.sample_rate);
        self.is_speaking = false;
        Some(segment)
    }
}

fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}
