use crate::config::SttConfig;
use crate::error::Result;

/// Whisper-based Speech-to-Text engine.
/// Phase 2: Will use whisper-rs with CUDA for GPU-accelerated transcription.
pub struct WhisperStt {
    _config: SttConfig,
}

impl WhisperStt {
    /// Initialize Whisper STT engine.
    /// TODO: Phase 2 — load model, create context with CUDA.
    pub fn new(_config: &SttConfig) -> Result<Self> {
        Ok(WhisperStt {
            _config: _config.clone(),
        })
    }

    /// Transcribe a speech segment (PCM f32, 16kHz mono).
    /// Returns the transcribed text.
    /// TODO: Phase 2 — actual whisper inference.
    pub fn transcribe(&self, _audio: &[f32]) -> Result<String> {
        Ok(String::new())
    }
}
