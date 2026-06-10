//! Streaming neural TTS using a Piper (VITS) model via sherpa-onnx.
//!
//! Synthesis runs on a dedicated thread. Generated audio is pushed into a
//! playback ring buffer through sherpa's streaming callback, so sound starts
//! as soon as the first chunk is ready — no per-utterance process warmup like
//! the SAPI backend. The speaker plays through the default output device.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::DeviceTrait;
use crossbeam_channel::{bounded, Sender};
use rtrb::{PushError, RingBuffer};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsVitsModelConfig,
};
use tracing::{error, info};

use crate::audio::{device, playback};
use crate::config::TtsConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::tts::Speaker;

/// Speaks text aloud via a Piper VITS model, streaming audio to the speakers.
pub struct PiperSpeaker {
    tx: Sender<String>,
}

impl PiperSpeaker {
    pub fn new(cfg: &TtsConfig) -> Result<Self> {
        let model = cfg.piper_model_path.clone();
        let tokens = cfg.piper_tokens_path.clone();
        let data_dir = cfg.piper_data_dir.clone();
        let out_name = cfg.output_device.clone();
        // Map the SAPI-style rate (~-10..10) to a Piper speed multiplier.
        let speed = (1.0 + cfg.rate as f32 * 0.05).clamp(0.5, 2.0);

        let (tx, rx) = bounded::<String>(64);
        // Propagate init success/failure (model load, audio device) back to caller.
        let (ready_tx, ready_rx) = bounded::<Result<u32>>(1);

        thread::Builder::new()
            .name("tts-piper".into())
            .spawn(move || {
                let tts_config = OfflineTtsConfig {
                    model: OfflineTtsModelConfig {
                        vits: OfflineTtsVitsModelConfig {
                            model: Some(model),
                            tokens: Some(tokens),
                            data_dir: Some(data_dir),
                            ..Default::default()
                        },
                        num_threads: 2,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let tts = match OfflineTts::create(&tts_config) {
                    Some(t) => t,
                    None => {
                        let _ = ready_tx.send(Err(VoiceTranslatorError::Tts(
                            "Failed to create Piper OfflineTts (check model/tokens/data_dir paths)"
                                .into(),
                        )));
                        return;
                    }
                };
                let model_sr = tts.sample_rate() as u32;

                // Route to a named device (e.g. "CABLE Input" virtual mic) or
                // fall back to the default speakers.
                let out_dev = if out_name.trim().is_empty() {
                    device::default_output_device()
                } else {
                    device::find_output_device_by_name(&out_name)
                };
                let out_dev = match out_dev {
                    Ok(d) => d,
                    Err(e) => { let _ = ready_tx.send(Err(e)); return; }
                };
                // The device often rejects the model's native 22050Hz/mono
                // (Realtek wants 48000Hz/stereo), so play in the device's
                // default format and resample/upmix TTS audio to match it.
                let dev_cfg = match out_dev.default_output_config() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = ready_tx.send(Err(VoiceTranslatorError::Audio(format!(
                            "No default output config: {e}"
                        ))));
                        return;
                    }
                };
                let out_sr = dev_cfg.sample_rate().0;
                let out_ch = dev_cfg.channels();

                // ~30s of audio headroom; producer stays here, consumer feeds playback.
                let cap = out_sr as usize * out_ch as usize * 30;
                let (producer, consumer) = RingBuffer::<f32>::new(cap);
                let _stream = match playback::start_playback(&out_dev, out_sr, out_ch, consumer) {
                    Ok(s) => s,
                    Err(e) => { let _ = ready_tx.send(Err(e)); return; }
                };

                let producer = Arc::new(Mutex::new(producer));
                let _ = ready_tx.send(Ok(out_sr));
                info!(
                    "Piper TTS ready (model {model_sr}Hz -> out {out_sr}Hz {out_ch}ch, speed {speed:.2})"
                );

                // Synthesize each queued clause to the full PCM buffer, then
                // resample + upmix to the device format and push for playback.
                // (Whole-clause; clauses are short so latency stays low and we
                // avoid streaming-resample boundary state.)
                while let Ok(text) = rx.recv() {
                    let gen = GenerationConfig { speed, ..Default::default() };
                    let no_cb: Option<fn(&[f32], f32) -> bool> = None;
                    match tts.generate_with_config(&text, &gen, no_cb) {
                        Some(audio) => {
                            let frames = resample_upmix(audio.samples(), model_sr, out_sr, out_ch);
                            if let Ok(mut p) = producer.lock() {
                                for s in frames {
                                    let mut v = s;
                                    while let Err(PushError::Full(x)) = p.push(v) {
                                        v = x;
                                        thread::sleep(Duration::from_millis(2));
                                    }
                                }
                            }
                        }
                        None => error!("Piper TTS generation failed"),
                    }
                }
            })
            .map_err(|e| VoiceTranslatorError::Tts(format!("Failed to spawn Piper thread: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(_sr)) => Ok(PiperSpeaker { tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(VoiceTranslatorError::Tts("Piper init thread died".into())),
        }
    }
}

impl Speaker for PiperSpeaker {
    fn speak(&self, text: &str) {
        // Drop on a full queue rather than block the translation thread.
        let _ = self.tx.try_send(text.to_string());
    }
}

/// Linearly resample mono `samples` from `from_sr` to `to_sr` and duplicate
/// each sample across `channels`, returning interleaved frames ready for an
/// output stream of that format.
fn resample_upmix(samples: &[f32], from_sr: u32, to_sr: u32, channels: u16) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let ratio = to_sr as f64 / from_sr as f64;
    let out_frames = ((samples.len() as f64) * ratio).round() as usize;
    let ch = channels.max(1) as usize;
    let last = samples.len() - 1;
    let mut out = Vec::with_capacity(out_frames * ch);
    for i in 0..out_frames {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = samples[idx.min(last)];
        let b = samples[(idx + 1).min(last)];
        let s = a + (b - a) * frac;
        for _ in 0..ch {
            out.push(s);
        }
    }
    out
}
