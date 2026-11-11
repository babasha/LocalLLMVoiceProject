use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::error::{Result, VoiceTranslatorError};

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub audio: AudioConfig,
    pub languages: LanguageConfig,
    pub vad: VadConfig,
    pub stt: SttConfig,
    /// STT for the "listen" direction (the other party's speech captured via
    /// loopback). Optional: without it the listen/duplex modes are unavailable.
    #[serde(default)]
    pub stt_listen: Option<SttConfig>,
    pub translation: TranslationConfig,
    pub tts: TtsConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub ring_buffer_capacity: usize,
    /// Microphone name substring. Empty = system default microphone.
    #[serde(default)]
    pub input_device: String,
    /// Output device whose sound is captured (WASAPI loopback) in the
    /// listen/duplex modes. Empty = system default output (speakers/headphones).
    #[serde(default)]
    pub loopback_device: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LanguageConfig {
    /// What you speak (mic direction source).
    pub source: String,
    /// What the listener hears (mic direction target).
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
    /// Recognizer family:
    ///   "nemo_ctc"        — single-file NeMo CTC (GigaAM-v3 e2e). Uses `model_path`.
    ///   "nemo_transducer" — NeMo TDT/RNNT (Parakeet). Uses encoder/decoder/joiner.
    ///   "whisper"         — Whisper encoder/decoder.
    #[serde(default = "default_stt_engine")]
    pub engine: String,
    #[serde(default)]
    pub model_path: String,
    pub tokens_path: String,
    #[serde(default)]
    pub encoder_path: String,
    #[serde(default)]
    pub decoder_path: String,
    #[serde(default)]
    pub joiner_path: String,
    #[serde(default)]
    pub language: String,
    #[serde(default = "default_stt_threads")]
    pub num_threads: i32,
    #[serde(default)]
    pub filter: SttFilterConfig,
}

fn default_stt_engine() -> String {
    "nemo_ctc".into()
}
fn default_stt_threads() -> i32 {
    4
}

/// Garbage / hallucination filter applied to every STT result.
#[derive(Debug, Deserialize, Clone)]
pub struct SttFilterConfig {
    /// Segments quieter than this RMS are treated as noise and not transcribed.
    #[serde(default = "default_min_rms")]
    pub min_rms: f32,
    /// Collapse a word/phrase repeated more than this many times in a row.
    #[serde(default = "default_max_repeats")]
    pub max_repeats: usize,
    /// Extra phrases to drop (case/punctuation-insensitive), on top of the
    /// built-in list of well-known Whisper-style hallucinations.
    #[serde(default)]
    pub blocklist: Vec<String>,
}

fn default_min_rms() -> f32 {
    0.004
}
fn default_max_repeats() -> usize {
    3
}

impl Default for SttFilterConfig {
    fn default() -> Self {
        SttFilterConfig {
            min_rms: default_min_rms(),
            max_repeats: default_max_repeats(),
            blocklist: Vec::new(),
        }
    }
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
    /// Chat-template family: "chatml"/"qwen" (Qwen `<|im_start|>` + `<think>`) or
    /// "gemma" (`<start_of_turn>` turns, no system role). Empty = auto-detect
    /// from the model filename so swapping `model_path` "just works".
    #[serde(default)]
    pub prompt_style: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TtsConfig {
    /// "piper" (neural, via sherpa-onnx) or "sapi" (built-in Windows voice).
    pub engine: String,
    /// Piper VITS model (.onnx).
    #[serde(default)]
    pub piper_model_path: String,
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

/// Back-pressure settings for the STT → translation hand-off.
#[derive(Debug, Deserialize, Clone)]
pub struct PipelineConfig {
    /// If the oldest pending utterance waited longer than this, a lag warning
    /// is emitted (console + UI).
    #[serde(default = "default_lag_warn")]
    pub lag_warn_ms: u64,
    /// When several finished utterances are queued, merge them into one LLM
    /// call (no speech is dropped, but the backlog is cleared in one pass).
    #[serde(default = "default_true")]
    pub merge_backlog: bool,
    /// Utterances that waited longer than this are shown untranslated and not
    /// spoken — late speech is worse than no speech in a live conversation.
    /// 0 = never drop.
    #[serde(default = "default_drop_after")]
    pub drop_after_ms: u64,
}

fn default_lag_warn() -> u64 {
    2500
}
fn default_drop_after() -> u64 {
    12000
}
fn default_true() -> bool {
    true
}

impl Default for PipelineConfig {
    fn default() -> Self {
        PipelineConfig {
            lag_warn_ms: default_lag_warn(),
            merge_backlog: true,
            drop_after_ms: default_drop_after(),
        }
    }
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
        config.resolve_paths(&project_root_for(path));
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

    /// Make every relative model path absolute against the project root, so
    /// the config is portable (no hard-coded `C:/...`) and works from any cwd.
    pub fn resolve_paths(&mut self, root: &Path) {
        let fix = |p: &mut String| {
            if !p.is_empty() && Path::new(p.as_str()).is_relative() {
                *p = root.join(p.as_str()).to_string_lossy().replace('\\', "/");
            }
        };
        fix(&mut self.vad.model_path);
        for stt in std::iter::once(&mut self.stt).chain(self.stt_listen.as_mut()) {
            fix(&mut stt.model_path);
            fix(&mut stt.tokens_path);
            fix(&mut stt.encoder_path);
            fix(&mut stt.decoder_path);
            fix(&mut stt.joiner_path);
        }
        fix(&mut self.translation.model_path);
        fix(&mut self.tts.piper_model_path);
        fix(&mut self.tts.piper_tokens_path);
        fix(&mut self.tts.piper_data_dir);
    }

    pub fn load_default() -> Result<Self> {
        let path = Path::new("config/default.toml");
        Self::load(path)
    }
}

/// The project root is the directory that contains `config/`. For a config
/// outside such a layout, paths are resolved next to the config file itself.
fn project_root_for(config_path: &Path) -> PathBuf {
    let abs = if config_path.is_absolute() {
        config_path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(config_path)
    };
    let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    match dir.file_name().and_then(|n| n.to_str()) {
        Some("config") => dir.parent().map(Path::to_path_buf).unwrap_or(dir),
        _ => dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_parent_of_config_dir() {
        let base = std::env::temp_dir();
        let r = project_root_for(&base.join("proj").join("config").join("default.toml"));
        assert_eq!(r, base.join("proj"));
        let r = project_root_for(&base.join("elsewhere").join("my.toml"));
        assert_eq!(r, base.join("elsewhere"));
    }
}
