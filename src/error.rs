use thiserror::Error;

#[derive(Error, Debug)]
pub enum VoiceTranslatorError {
    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("VAD error: {0}")]
    Vad(String),

    #[error("STT error: {0}")]
    Stt(String),

    #[error("Translation error: {0}")]
    Translation(String),

    #[error("TTS error: {0}")]
    Tts(String),

    #[error("Pipeline error: {0}")]
    Pipeline(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, VoiceTranslatorError>;
