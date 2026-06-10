pub mod kokoro;
pub mod phonemize;
pub mod piper;
pub mod sapi;

use crate::config::TtsConfig;
use crate::error::{Result, VoiceTranslatorError};

/// Trait for TTS engines that synthesize text to PCM f32 audio (offline,
/// whole-utterance). Kept for the placeholder engines.
pub trait TtsEngine: Send + 'static {
    /// Synthesize text to PCM f32 audio.
    fn synthesize(&self, text: &str) -> Result<Vec<f32>>;

    /// Output sample rate of the synthesized audio.
    fn sample_rate(&self) -> u32;
}

/// A speaker plays translated text aloud. Implementations own their own audio
/// output and synthesize asynchronously, so `speak` returns promptly.
pub trait Speaker: Send {
    /// Queue `text` to be spoken aloud.
    fn speak(&self, text: &str);
}

/// Build a speaker from config. `cfg.engine` selects the backend:
///   "piper" — streaming neural Piper (VITS) via sherpa-onnx (recommended)
///   "sapi"  — built-in Windows SAPI voice (no models, but per-utterance warmup)
pub fn create_speaker(cfg: &TtsConfig) -> Result<Box<dyn Speaker>> {
    match cfg.engine.as_str() {
        "piper" => Ok(Box::new(piper::PiperSpeaker::new(cfg)?)),
        "sapi" => Ok(Box::new(sapi::SapiSpeaker::new(cfg.rate)?)),
        other => Err(VoiceTranslatorError::Tts(format!(
            "Unknown TTS engine for speaking: '{other}' (use \"piper\" or \"sapi\")"
        ))),
    }
}
