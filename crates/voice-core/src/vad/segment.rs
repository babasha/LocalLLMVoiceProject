use std::time::Instant;

/// A segment of detected speech audio.
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    /// Raw PCM audio data (f32, mono, 16kHz)
    pub audio: Vec<f32>,
    /// When the speech started
    pub timestamp: Instant,
    /// Duration in seconds
    pub duration_secs: f32,
}

impl SpeechSegment {
    pub fn new(audio: Vec<f32>, sample_rate: u32) -> Self {
        let duration_secs = audio.len() as f32 / sample_rate as f32;
        SpeechSegment {
            audio,
            timestamp: Instant::now(),
            duration_secs,
        }
    }
}
