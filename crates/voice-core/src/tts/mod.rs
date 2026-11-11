pub mod piper;
pub mod sapi;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::TtsConfig;
use crate::error::{Result, VoiceTranslatorError};

/// A speaker plays translated text aloud. Implementations own their own audio
/// output and synthesize asynchronously, so `speak` returns promptly.
pub trait Speaker: Send {
    /// Queue `text` to be spoken aloud.
    fn speak(&self, text: &str);
}

/// Tracks until when our own TTS is (probably) audible. In duplex mode the
/// loopback channel ignores audio while the gate is active, so the translator
/// does not hear — and re-translate — its own voice when the TTS plays through
/// the same device that is being loopback-captured.
pub struct EchoGate {
    epoch: Instant,
    /// Milliseconds since `epoch` until which playback is expected to last.
    until_ms: AtomicU64,
    /// Extra time after playback ends (device latency + room tail).
    tail_ms: u64,
}

impl EchoGate {
    pub fn new() -> Arc<Self> {
        Arc::new(EchoGate { epoch: Instant::now(), until_ms: AtomicU64::new(0), tail_ms: 350 })
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    /// Audio of length `d` was queued for playback right after whatever is
    /// already queued.
    pub fn extend(&self, d: Duration) {
        let now = self.now_ms();
        let _ = self.until_ms.fetch_update(Ordering::AcqRel, Ordering::Acquire, |u| {
            Some(u.max(now) + (d.as_millis() as u64).max(1))
        });
    }

    pub fn active(&self) -> bool {
        // 0 = nothing has been played yet.
        let until = self.until_ms.load(Ordering::Acquire);
        until > 0 && self.now_ms() < until + self.tail_ms
    }
}

/// Build a speaker from config. `cfg.engine` selects the backend:
///   "piper" — streaming neural Piper (VITS) via sherpa-onnx (recommended)
///   "sapi"  — built-in Windows SAPI voice (no models, but per-utterance warmup)
pub fn create_speaker(cfg: &TtsConfig, gate: Option<Arc<EchoGate>>) -> Result<Box<dyn Speaker>> {
    match cfg.engine.as_str() {
        "piper" => Ok(Box::new(piper::PiperSpeaker::new(cfg, gate)?)),
        "sapi" => Ok(Box::new(sapi::SapiSpeaker::new(cfg.rate, gate)?)),
        other => Err(VoiceTranslatorError::Tts(format!(
            "Unknown TTS engine for speaking: '{other}' (use \"piper\" or \"sapi\")"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_activates_and_expires() {
        let g = EchoGate::new();
        assert!(!g.active());
        g.extend(Duration::from_millis(50));
        assert!(g.active());
        std::thread::sleep(Duration::from_millis(50 + 400));
        assert!(!g.active());
    }
}
