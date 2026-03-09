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
    pub threshold: f32,
    pub silence_duration_ms: u32,
    pub min_speech_duration_ms: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SttConfig {
    pub model_path: String,
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
}

#[derive(Debug, Deserialize, Clone)]
pub struct TtsConfig {
    pub engine: String,
    pub kokoro_model_path: String,
    pub piper_model_path: String,
    #[serde(default = "default_voice")]
    pub kokoro_voice: String,
    pub output_sample_rate: u32,
}

fn default_voice() -> String {
    "af_heart".to_string()
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            VoiceTranslatorError::Config(format!("Failed to read config {}: {}", path.display(), e))
        })?;
        let config: AppConfig = toml::from_str(&content).map_err(|e| {
            VoiceTranslatorError::Config(format!("Failed to parse config: {}", e))
        })?;
        Ok(config)
    }

    pub fn load_default() -> Result<Self> {
        let path = Path::new("config/default.toml");
        Self::load(path)
    }
}
