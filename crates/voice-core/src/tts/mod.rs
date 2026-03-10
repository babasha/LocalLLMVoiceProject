pub mod kokoro;
pub mod phonemize;
pub mod piper;

use crate::error::Result;

/// Trait for TTS engines. Implementations must be Send for use across threads.
pub trait TtsEngine: Send + 'static {
    /// Synthesize text to PCM f32 audio.
    fn synthesize(&self, text: &str) -> Result<Vec<f32>>;

    /// Output sample rate of the synthesized audio.
    fn sample_rate(&self) -> u32;
}

/// Create a TTS engine based on the engine name in config.
pub fn create_engine(engine_name: &str) -> Result<Box<dyn TtsEngine>> {
    match engine_name {
        "kokoro" => {
            tracing::info!("Using Kokoro TTS engine (placeholder)");
            Ok(Box::new(kokoro::KokoroTts::new()?))
        }
        "piper" => {
            tracing::info!("Using Piper TTS engine (placeholder)");
            Ok(Box::new(piper::PiperTts::new()?))
        }
        other => Err(crate::error::VoiceTranslatorError::Tts(format!(
            "Unknown TTS engine: {}",
            other
        ))),
    }
}
