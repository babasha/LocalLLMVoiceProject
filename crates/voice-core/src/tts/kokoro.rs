use crate::error::Result;
use crate::tts::TtsEngine;

/// Kokoro TTS engine (82M parameters, ONNX + CUDA).
/// Phase 4: Will use kokoro-tts or kokorox crate with ort CUDA backend.
pub struct KokoroTts;

impl KokoroTts {
    pub fn new() -> Result<Self> {
        Ok(KokoroTts)
    }
}

impl TtsEngine for KokoroTts {
    fn synthesize(&self, _text: &str) -> Result<Vec<f32>> {
        // TODO: Phase 4 — actual Kokoro inference
        Ok(Vec::new())
    }

    fn sample_rate(&self) -> u32 {
        24000
    }
}
