use std::time::Instant;

use crate::vad::segment::SpeechSegment;

/// Message from VAD → STT: a complete speech segment to transcribe.
#[derive(Debug, Clone)]
pub struct VadToStt {
    pub segment: SpeechSegment,
}

/// Message from STT → Translation: transcribed text.
#[derive(Debug, Clone)]
pub struct SttToTranslation {
    pub text: String,
    pub source_lang: String,
    pub timestamp: Instant,
}

/// Message from Translation → TTS: translated text chunk.
#[derive(Debug, Clone)]
pub struct TranslationToTts {
    pub text: String,
    pub target_lang: String,
    pub timestamp: Instant,
}

/// Message from TTS → Playback: synthesized audio.
#[derive(Debug, Clone)]
pub struct TtsToPlayback {
    pub audio: Vec<f32>,
    pub sample_rate: u32,
    pub timestamp: Instant,
}
