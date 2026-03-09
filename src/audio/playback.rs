use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::StreamConfig;
use rtrb::Consumer;
use tracing::{error, info};

use crate::error::{Result, VoiceTranslatorError};

/// Builds and starts an audio output stream that reads f32 samples from the ring buffer.
///
/// Outputs silence when the ring buffer is empty (no blocking).
/// Returns the Stream handle (must be kept alive to continue playback).
pub fn start_playback(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u16,
    mut consumer: Consumer<f32>,
) -> Result<cpal::Stream> {
    let config = StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    info!(
        "Starting playback on '{}': {}Hz, {} ch",
        device_name, sample_rate, channels
    );

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                // Zero-alloc: read from lock-free ring buffer, fill silence if empty
                for sample in data.iter_mut() {
                    *sample = consumer.pop().unwrap_or(0.0);
                }
            },
            move |err| {
                error!("Audio playback error: {}", err);
            },
            None,
        )
        .map_err(|e| {
            VoiceTranslatorError::Audio(format!("Failed to build output stream: {}", e))
        })?;

    stream
        .play()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to start playback: {}", e)))?;

    info!("Audio playback started");
    Ok(stream)
}

/// Creates a dual-output setup: primary speakers + VB-Cable.
/// Both output streams read from separate ring buffers that need to be fed
/// with the same data by the caller.
pub struct DualOutput {
    pub primary_stream: cpal::Stream,
    pub secondary_stream: Option<cpal::Stream>,
}

impl DualOutput {
    pub fn new(
        primary_device: &cpal::Device,
        secondary_device: Option<&cpal::Device>,
        sample_rate: u32,
        channels: u16,
        primary_consumer: Consumer<f32>,
        secondary_consumer: Option<Consumer<f32>>,
    ) -> Result<Self> {
        let primary_stream = start_playback(primary_device, sample_rate, channels, primary_consumer)?;

        let secondary_stream = match (secondary_device, secondary_consumer) {
            (Some(device), Some(consumer)) => {
                Some(start_playback(device, sample_rate, channels, consumer)?)
            }
            _ => None,
        };

        Ok(DualOutput {
            primary_stream,
            secondary_stream,
        })
    }
}
