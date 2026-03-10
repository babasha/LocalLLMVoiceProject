use cpal::traits::{DeviceTrait, HostTrait};
use tracing::info;

use crate::error::{Result, VoiceTranslatorError};

/// Lists all available audio input devices.
pub fn list_input_devices() -> Result<Vec<(String, cpal::Device)>> {
    let host = cpal::default_host();
    let devices: Vec<_> = host
        .input_devices()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to enumerate input devices: {}", e)))?
        .filter_map(|d| {
            d.name().ok().map(|name| (name, d))
        })
        .collect();
    Ok(devices)
}

/// Lists all available audio output devices.
pub fn list_output_devices() -> Result<Vec<(String, cpal::Device)>> {
    let host = cpal::default_host();
    let devices: Vec<_> = host
        .output_devices()
        .map_err(|e| VoiceTranslatorError::Audio(format!("Failed to enumerate output devices: {}", e)))?
        .filter_map(|d| {
            d.name().ok().map(|name| (name, d))
        })
        .collect();
    Ok(devices)
}

/// Returns the default input device.
pub fn default_input_device() -> Result<cpal::Device> {
    let host = cpal::default_host();
    host.default_input_device()
        .ok_or_else(|| VoiceTranslatorError::DeviceNotFound("No default input device".into()))
}

/// Returns the default output device.
pub fn default_output_device() -> Result<cpal::Device> {
    let host = cpal::default_host();
    host.default_output_device()
        .ok_or_else(|| VoiceTranslatorError::DeviceNotFound("No default output device".into()))
}

/// Finds an output device by name substring (e.g., "VB-Cable" or "CABLE Input").
pub fn find_output_device_by_name(name_substr: &str) -> Result<cpal::Device> {
    let devices = list_output_devices()?;
    for (name, device) in &devices {
        if name.to_lowercase().contains(&name_substr.to_lowercase()) {
            info!("Found output device: {}", name);
            return Ok(device.clone());
        }
    }
    Err(VoiceTranslatorError::DeviceNotFound(format!(
        "No output device matching '{}'", name_substr
    )))
}

/// Prints all audio devices for debugging.
pub fn print_all_devices() -> Result<()> {
    println!("=== Input Devices ===");
    for (name, _) in list_input_devices()? {
        println!("  [IN]  {}", name);
    }
    println!("\n=== Output Devices ===");
    for (name, _) in list_output_devices()? {
        println!("  [OUT] {}", name);
    }
    Ok(())
}

/// Clone a device reference (cpal::Device is Clone on WASAPI).
/// Needed because we might want both speakers and VB-Cable output.
pub fn clone_device(device: &cpal::Device) -> cpal::Device {
    device.clone()
}
