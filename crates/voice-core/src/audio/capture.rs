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
