use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use rtrb::Producer;
use tracing::{error, info, warn};

use crate::error::{Result, VoiceTranslatorError};

/// Builds and starts an audio input stream that pushes f32 samples into the ring buffer.
///
/// The callback runs at real-time priority (WASAPI thread) — no allocations allowed.
/// Returns the Stream handle (must be kept alive to continue capturing).
pub fn start_capture(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u16,
    mut producer: Producer<f32>,
) -> Result<cpal::Stream> {
    let config = StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let supported = device
        .supported_input_configs()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to query input configs: {}", e)))?;

    // Log supported configs for debugging
    for cfg in supported {
        info!(
            "Supported input: channels={}, sample_rate={}-{}, format={:?}",
            cfg.channels(),
            cfg.min_sample_rate().0,
            cfg.max_sample_rate().0,
            cfg.sample_format(),
        );
    }

    let device_name = device
        .name()
        .unwrap_or_else(|_| "unknown".to_string());
    info!(
        "Starting capture on '{}': {}Hz, {} ch",
        device_name, sample_rate, channels
    );

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                // Zero-alloc: just push samples into lock-free ring buffer
                for &sample in data {
                    if producer.push(sample).is_err() {
                        // Ring buffer full — drop sample (better than blocking)
                        // In production, this indicates consumer is too slow
                    }
                }
            },
            move |err| {
                error!("Audio capture error: {}", err);
            },
            None,
        )
        .map_err(|e| {
            VoiceTranslatorError::Audio(format!("Failed to build input stream: {}", e))
        })?;

    stream
        .play()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to start capture: {}", e)))?;

    info!("Audio capture started");
    Ok(stream)
}

/// Real-time box-average downsampler state, carried across cpal callbacks.
/// Averages ~`samples_per_out` input samples per emitted output sample, which
/// both converts the rate and acts as a cheap anti-aliasing low-pass.
struct Downsampler {
    samples_per_out: f32,
    sum: f32,
    count: f32,
}

impl Downsampler {
    fn new(native_rate: u32, target_rate: u32) -> Self {
        Downsampler {
            samples_per_out: (native_rate as f32 / target_rate as f32).max(1.0),
            sum: 0.0,
            count: 0.0,
        }
    }

    #[inline]
    fn push_mono(&mut self, mono: f32, producer: &mut Producer<f32>) {
        self.sum += mono;
        self.count += 1.0;
        if self.count >= self.samples_per_out {
            // Ring buffer full → drop (better than blocking the audio thread).
            let _ = producer.push(self.sum / self.count);
            self.sum = 0.0;
            self.count -= self.samples_per_out;
        }
    }
}

/// Capture from the device at its native rate/format, downmix to mono and
/// resample to `target_rate`, pushing mono f32 samples into the ring buffer.
///
/// Many devices (e.g. Realtek on Windows/WASAPI) only support 48 kHz stereo,
/// so requesting 16 kHz mono directly fails. This negotiates the device's
/// default config and converts in the callback — real-time safe (no alloc).
pub fn start_capture_converted(
    device: &cpal::Device,
    target_rate: u32,
    mut producer: Producer<f32>,
) -> Result<cpal::Stream> {
    let default_cfg = device.default_input_config().map_err(|e| {
        VoiceTranslatorError::Audio(format!("No default input config: {}", e))
    })?;
    let native_rate = default_cfg.sample_rate().0;
    let channels = default_cfg.channels() as usize;
    let sample_format = default_cfg.sample_format();
    let config: StreamConfig = default_cfg.into();

    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    info!(
        "Capture '{}': native {}Hz {}ch {:?} → {}Hz mono",
        device_name, native_rate, channels, sample_format, target_rate
    );

    let mut ds = Downsampler::new(native_rate, target_rate);
    let err_fn = |err| error!("Audio capture error: {}", err);

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mut i = 0;
                while i + channels <= data.len() {
                    let mut acc = 0.0f32;
                    for c in 0..channels {
                        acc += data[i + c];
                    }
                    ds.push_mono(acc / channels as f32, &mut producer);
                    i += channels;
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let mut i = 0;
                while i + channels <= data.len() {
                    let mut acc = 0.0f32;
                    for c in 0..channels {
                        acc += data[i + c] as f32 / 32768.0;
                    }
                    ds.push_mono(acc / channels as f32, &mut producer);
                    i += channels;
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I32 => device.build_input_stream(
            &config,
            move |data: &[i32], _: &cpal::InputCallbackInfo| {
                let mut i = 0;
                while i + channels <= data.len() {
                    let mut acc = 0.0f32;
                    for c in 0..channels {
                        acc += data[i + c] as f32 / 2_147_483_648.0;
                    }
                    ds.push_mono(acc / channels as f32, &mut producer);
                    i += channels;
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::U8 => device.build_input_stream(
            &config,
            move |data: &[u8], _: &cpal::InputCallbackInfo| {
                let mut i = 0;
                while i + channels <= data.len() {
                    let mut acc = 0.0f32;
                    for c in 0..channels {
                        acc += (data[i + c] as f32 - 128.0) / 128.0;
                    }
                    ds.push_mono(acc / channels as f32, &mut producer);
                    i += channels;
                }
            },
            err_fn,
            None,
        ),
        other => {
            return Err(VoiceTranslatorError::Audio(format!(
                "Unsupported input sample format: {:?}",
                other
            )));
        }
    }
    .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to build input stream: {}", e)))?;

    stream
        .play()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to start capture: {}", e)))?;

    info!("Audio capture started (converted to {}Hz mono)", target_rate);
    Ok(stream)
}

/// Builds an input stream that captures raw bytes and converts to f32.
/// Handles I16 and F32 sample formats from the device.
pub fn start_capture_adaptive(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u16,
    mut producer: Producer<f32>,
) -> Result<cpal::Stream> {
    // Find a supported config matching our requirements
    let supported_configs: Vec<_> = device
        .supported_input_configs()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to query input configs: {}", e)))?
        .collect();

    // Prefer F32, fall back to I16
    let (format, config) = find_best_config(&supported_configs, sample_rate, channels)?;

    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    info!(
        "Starting adaptive capture on '{}': {}Hz, {} ch, {:?}",
        device_name, sample_rate, channels, format
    );

    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                for &sample in data {
                    let _ = producer.push(sample);
                }
            },
            |err| error!("Capture error: {}", err),
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                for &sample in data {
                    let f = sample as f32 / i16::MAX as f32;
                    let _ = producer.push(f);
                }
            },
            |err| error!("Capture error: {}", err),
            None,
        ),
        _ => {
            return Err(VoiceTranslatorError::Audio(format!(
                "Unsupported sample format: {:?}",
                format
            )));
        }
    }
    .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to build input stream: {}", e)))?;

    stream
        .play()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to start capture: {}", e)))?;

    info!("Audio capture started (adaptive)");
    Ok(stream)
}

fn find_best_config(
    configs: &[cpal::SupportedStreamConfigRange],
    sample_rate: u32,
    channels: u16,
) -> Result<(SampleFormat, StreamConfig)> {
    let target_rate = cpal::SampleRate(sample_rate);

    // First try: exact match with F32
    for cfg in configs {
        if cfg.channels() == channels
            && cfg.min_sample_rate() <= target_rate
            && cfg.max_sample_rate() >= target_rate
            && cfg.sample_format() == SampleFormat::F32
        {
            return Ok((
                SampleFormat::F32,
                cfg.with_sample_rate(target_rate).into(),
            ));
        }
    }

    // Second try: any format, matching channels and rate
    for cfg in configs {
        if cfg.channels() == channels
            && cfg.min_sample_rate() <= target_rate
            && cfg.max_sample_rate() >= target_rate
        {
            return Ok((
                cfg.sample_format(),
                cfg.with_sample_rate(target_rate).into(),
            ));
        }
    }

    // Third try: any config with matching channels, use max supported rate
    // (will need resampling)
    for cfg in configs {
        if cfg.channels() == channels {
            let rate = cfg.max_sample_rate();
            warn!(
                "Using {}Hz instead of target {}Hz — resampling may be needed",
                rate.0, sample_rate
            );
            return Ok((cfg.sample_format(), cfg.with_sample_rate(rate).into()));
        }
    }

    Err(VoiceTranslatorError::Audio(
        "No suitable input config found".into(),
    ))
}
