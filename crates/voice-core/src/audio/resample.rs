use rubato::{
    FftFixedIn, Resampler,
};
use tracing::info;

use crate::error::{Result, VoiceTranslatorError};

/// Resamples a mono f32 audio buffer from `from_rate` to `to_rate`.
/// Returns the resampled buffer.
pub fn resample_mono(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>> {
    if from_rate == to_rate {
        return Ok(input.to_vec());
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024;

    let mut resampler = FftFixedIn::<f64>::new(from_rate as usize, to_rate as usize, chunk_size, 2, 1).map_err(|e| {
        VoiceTranslatorError::Audio(format!("Failed to create resampler: {}", e))
    })?;

    info!(
        "Resampling: {}Hz → {}Hz ({} samples)",
        from_rate,
        to_rate,
        input.len()
    );

    let mut output = Vec::with_capacity((input.len() as f64 * ratio) as usize + chunk_size);

    // Process in chunks
    let mut pos = 0;
    while pos < input.len() {
        let end = (pos + chunk_size).min(input.len());
        let mut chunk: Vec<f64> = input[pos..end].iter().map(|&s| s as f64).collect();

        // Pad last chunk if needed
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }

        let input_frames = vec![chunk];
        let resampled = resampler.process(&input_frames, None).map_err(|e| {
            VoiceTranslatorError::Audio(format!("Resampling failed: {}", e))
        })?;

        for &sample in &resampled[0] {
            output.push(sample as f32);
        }

        pos += chunk_size;
    }

    // Trim to expected length
    let expected_len = (input.len() as f64 * ratio) as usize;
    output.truncate(expected_len);

    Ok(output)
}
