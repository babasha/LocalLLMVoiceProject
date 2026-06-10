use serde::Deserialize;
use std::path::Path;

use crate::error::{Result, VoiceTranslatorError};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub audio: AudioConfig,
    pub languages: LanguageConfig,
    pub vad: VadConfig,
    pub stt: SttConfig,
    pub translation: TranslationConfig,
    pub tts: TtsConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub ring_buffer_capacity: usize,
    #[serde(default)]
    pub vb_cable_device: String,
    #[serde(default)]
    pub dual_output: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LanguageConfig {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct VadConfig {
    pub model_path: String,
    pub threshold: f32,
    pub silence_duration_ms: u32,
    pub min_speech_duration_ms: u32,
    #[serde(default = "default_max_speech")]
    pub max_speech_duration_ms: u32,
}

fn default_max_speech() -> u32 {
    15000
}

#[derive(Debug, Deserialize, Clone)]
pub struct SttConfig {
    pub model_path: String,
    pub tokens_path: String,
    #[serde(default)]
    pub language: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TranslationConfig {
    pub model_path: String,
    pub n_gpu_layers: u32,
    pub context_size: u32,
    pub max_tokens: u32,
    pub temperature: f32,
    #[serde(default)]
    pub enable_thinking: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TtsConfig {
    pub engine: String,
    pub kokoro_model_path: String,
    pub piper_model_path: String,
    #[serde(default = "default_voice")]
    pub kokoro_voice: String,
    pub output_sample_rate: u32,
    /// Speak the translated output aloud. Off by default.
    #[serde(default)]
    pub speak: bool,
    /// Speech rate. SAPI: -10 (slow) .. 10 (fast), 0 = normal.
    /// Piper maps this to a speed multiplier (1.0 + rate*0.05).
    #[serde(default)]
    pub rate: i32,
    /// Piper tokens.txt (phoneme->id map) for the VITS model.
    #[serde(default)]
    pub piper_tokens_path: String,
    /// Piper espeak-ng-data directory (phonemization data).
    #[serde(default)]
    pub piper_data_dir: String,
    /// Output device name substring for spoken audio. Empty = default speakers.
    /// Set to "CABLE Input" to feed a VB-CABLE virtual microphone (for Telegram etc.).
    #[serde(default)]
    pub output_device: String,
}

fn default_voice() -> String {
    "af_heart".to_string()
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            VoiceTranslatorError::Config(format!("Failed to read config {}: {}", path.display(), e))
        })?;
        let mut config: AppConfig = toml::from_str(&content).map_err(|e| {
            VoiceTranslatorError::Config(format!("Failed to parse config: {}", e))
        })?;
        config.apply_env_overrides();
        Ok(config)
    }

    /// Let the launcher/GUI override TTS settings without editing the TOML.
    ///   VOICE_TTS_OUTPUT — output device name substring ("" = default speakers)
    ///   VOICE_TTS_SPEAK  — "1"/"true"/"on" to enable, anything else to disable
    ///   VOICE_TTS_ENGINE — "piper" | "sapi"
    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("VOICE_TTS_OUTPUT") {
            self.tts.output_device = v;
        }
        if let Ok(v) = std::env::var("VOICE_TTS_SPEAK") {
            self.tts.speak = matches!(v.to_lowercase().as_str(), "1" | "true" | "on" | "yes");
        }
        if let Ok(v) = std::env::var("VOICE_TTS_ENGINE") {
            if !v.is_empty() {
                self.tts.engine = v;
            }
        }
    }

    pub fn load_default() -> Result<Self> {
        let path = Path::new("config/default.toml");
        Self::load(path)
    }
}
