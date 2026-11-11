pub mod filter;

use sherpa_onnx::{
    OfflineNemoEncDecCtcModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineTransducerModelConfig, OfflineWhisperModelConfig,
};
use tracing::info;

use crate::config::SttConfig;
use crate::error::{Result, VoiceTranslatorError};
use filter::SttFilter;

/// Offline speech recognizer (sherpa-onnx) for any of the supported model
/// families, with the garbage/hallucination filter applied to every result.
pub struct SpeechRecognizer {
    recognizer: OfflineRecognizer,
    filter: SttFilter,
    label: String,
}

impl SpeechRecognizer {
    pub fn new(config: &SttConfig) -> Result<Self> {
        let mut cfg = OfflineRecognizerConfig::default();
        let some = |s: &String| (!s.is_empty()).then(|| s.clone());

        let label = match config.engine.as_str() {
            "nemo_ctc" | "gigaam" | "" => {
                cfg.model_config.nemo_ctc = OfflineNemoEncDecCtcModelConfig {
                    model: some(&config.model_path),
                };
                format!("NeMo CTC ({})", file_name(&config.model_path))
            }
            "nemo_transducer" | "parakeet" | "transducer" => {
                cfg.model_config.transducer = OfflineTransducerModelConfig {
                    encoder: some(&config.encoder_path),
                    decoder: some(&config.decoder_path),
                    joiner: some(&config.joiner_path),
                };
                if config.engine != "transducer" {
                    cfg.model_config.model_type = Some("nemo_transducer".into());
                }
                format!("Transducer ({})", file_name(&config.encoder_path))
            }
            "whisper" => {
                cfg.model_config.whisper = OfflineWhisperModelConfig {
                    encoder: some(&config.encoder_path),
                    decoder: some(&config.decoder_path),
                    language: some(&config.language),
                    task: Some("transcribe".into()),
                    tail_paddings: -1,
                    ..Default::default()
                };
                format!("Whisper ({})", file_name(&config.encoder_path))
            }
            other => {
                return Err(VoiceTranslatorError::Stt(format!(
                    "Unknown STT engine '{other}' (use nemo_ctc | nemo_transducer | whisper)"
                )))
            }
        };
        cfg.model_config.tokens = Some(config.tokens_path.clone());
        cfg.model_config.num_threads = config.num_threads;

        info!("Loading STT: {label}");
        let recognizer = OfflineRecognizer::create(&cfg).ok_or_else(|| {
            VoiceTranslatorError::Stt(format!("Failed to create recognizer: {label}"))
        })?;
        info!("STT ready: {label}");

        Ok(SpeechRecognizer {
            recognizer,
            filter: SttFilter::new(&config.filter),
            label,
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Transcribe a speech segment (PCM f32, 16kHz mono). Returns an empty
    /// string for noise / known hallucinations.
    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        if !self.filter.audio_has_speech(audio) {
            return Ok(String::new());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16000, audio);
        self.recognizer.decode(&stream);

        let result = stream
            .get_result()
            .ok_or_else(|| VoiceTranslatorError::Stt("Recognizer returned no result".into()))?;

        Ok(self.filter.clean(&result.text))
    }
}

fn file_name(p: &str) -> &str {
    p.rsplit(['/', '\\']).next().unwrap_or(p)
}
