use crate::error::Result;
use crate::tts::TtsEngine;

/// Piper TTS engine (~15-20M parameters, ONNX + CUDA).
/// Phase 4: Will use piper-rs crate.
pub struct PiperTts;

impl PiperTts {
    pub fn new() -> Result<Self> {
        Ok(PiperTts)
    }
}

impl TtsEngine for PiperTts {
    fn synthesize(&self, _text: &str) -> Result<Vec<f32>> {
        // TODO: Phase 4 — actual Piper inference
        Ok(Vec::new())
    }

    fn sample_rate(&self) -> u32 {
        22050
    }
}
