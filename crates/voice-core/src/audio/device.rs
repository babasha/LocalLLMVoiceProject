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

/// Finds an input (capture) device by name substring.
pub fn find_input_device_by_name(name_substr: &str) -> Result<cpal::Device> {
    let needle = name_substr.to_lowercase();
    for (name, device) in list_input_devices()? {
        if name.to_lowercase().contains(&needle) {
            info!("Found input device: {}", name);
            return Ok(device);
        }
    }
    Err(VoiceTranslatorError::DeviceNotFound(format!(
        "No input device matching '{}'", name_substr
    )))
}

/// Microphone: named one, or the system default when `name` is empty.
pub fn microphone(name: &str) -> Result<cpal::Device> {
    if name.trim().is_empty() {
        default_input_device()
    } else {
        find_input_device_by_name(name)
    }
}

/// Output device to capture in loopback mode (what the other party says in a
/// call is played here): named one, or the system default output.
pub fn loopback_source(name: &str) -> Result<cpal::Device> {
    if name.trim().is_empty() {
        default_output_device()
    } else {
        find_output_device_by_name(name)
    }
}

/// True if two output selectors (name substrings, "" = default) refer to the
/// same physical device. Used to detect TTS → loopback echo.
pub fn same_output(a: &str, b: &str) -> bool {
    let resolve = |s: &str| -> Option<String> {
        if s.trim().is_empty() {
            default_output_device().ok()?.name().ok()
        } else {
            find_output_device_by_name(s).ok()?.name().ok()
        }
    };
    match (resolve(a), resolve(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim().eq_ignore_ascii_case(b.trim()),
    }
}

/// Machine-readable device list for the GUI:
/// one line per device, `IN\t<name>` / `OUT\t<name>`, defaults marked with `*`.
pub fn print_devices_tsv() -> Result<()> {
    let def_in = default_input_device().ok().and_then(|d| d.name().ok());
    let def_out = default_output_device().ok().and_then(|d| d.name().ok());
    for (name, _) in list_input_devices()? {
        let mark = if Some(&name) == def_in.as_ref() { "*" } else { "" };
        println!("IN{mark}\t{name}");
    }
    for (name, _) in list_output_devices()? {
        let mark = if Some(&name) == def_out.as_ref() { "*" } else { "" };
        println!("OUT{mark}\t{name}");
    }
    Ok(())
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
