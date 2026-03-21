use sherpa_onnx::{OfflineNemoEncDecCtcModelConfig, OfflineRecognizer, OfflineRecognizerConfig};
use tracing::info;

use crate::config::SttConfig;
use crate::error::{Result, VoiceTranslatorError};

/// GigaAM-v3 Speech-to-Text engine via sherpa-onnx (NeMo CTC e2e).
/// Uses INT8-quantized ONNX model with built-in punctuation and text normalization.
pub struct GigaAmStt {
    recognizer: OfflineRecognizer,
}

impl GigaAmStt {
    pub fn new(config: &SttConfig) -> Result<Self> {
        info!("Loading GigaAM-v3 model from {}", config.model_path);

        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.nemo_ctc = OfflineNemoEncDecCtcModelConfig {
            model: Some(config.model_path.clone()),
        };
        cfg.model_config.tokens = Some(config.tokens_path.clone());
        cfg.model_config.num_threads = 4;

        let recognizer = OfflineRecognizer::create(&cfg).ok_or_else(|| {
            VoiceTranslatorError::Stt("Failed to create GigaAM-v3 recognizer".into())
        })?;

        info!("GigaAM-v3 model loaded successfully");
        Ok(GigaAmStt { recognizer })
    }

    /// Transcribe a speech segment (PCM f32, 16kHz mono).
    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16000, audio);
        self.recognizer.decode(&stream);

        let result = stream
            .get_result()
            .ok_or_else(|| VoiceTranslatorError::Stt("GigaAM returned no result".into()))?;

        Ok(result.text)
    }
}
