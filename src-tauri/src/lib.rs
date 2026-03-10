use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tracing::info;

use voice_core::config::AppConfig;

/// Wrapper to make cpal::Stream usable across threads.
/// Safe because on ALSA the stream's audio callback runs in its own thread
/// and we only start/stop from the main thread.
struct SendStream(Option<cpal::Stream>);
unsafe impl Send for SendStream {}
unsafe impl Sync for SendStream {}

/// Audio monitoring state — captures mic level in real-time.
struct AudioMonitor {
    stream: SendStream,
    /// Peak level 0-1000 (scaled to avoid floats in atomics)
    peak_level: Arc<AtomicU32>,
    active_device: String,
}

/// Application state shared between Tauri commands.
struct AppState {
    config: Mutex<AppConfig>,
    is_running: Mutex<bool>,
    current_mode: Mutex<String>,
    selected_input: Mutex<String>,
    selected_output: Mutex<String>,
    monitor: Mutex<AudioMonitor>,
    monitoring_active: Arc<AtomicBool>,
}

#[derive(Serialize, Deserialize, Clone)]
struct StatusResponse {
    is_running: bool,
    mode: String,
    source_lang: String,
    target_lang: String,
    model_path: String,
    gpu_layers: u32,
    selected_input: String,
    selected_output: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct AudioDevice {
    name: String,
    is_input: bool,
    is_default: bool,
    is_selected: bool,
}

#[derive(Deserialize)]
struct TranslateRequest {
    text: String,
    source_lang: String,
    target_lang: String,
}

#[derive(Serialize)]
struct TranslateResponse {
    translation: String,
}

// --- Tauri Commands ---

#[tauri::command]
fn get_status(state: State<AppState>) -> StatusResponse {
    let config = state.config.lock().unwrap();
    let is_running = *state.is_running.lock().unwrap();
    let mode = state.current_mode.lock().unwrap().clone();
    let selected_input = state.selected_input.lock().unwrap().clone();
    let selected_output = state.selected_output.lock().unwrap().clone();

    StatusResponse {
        is_running,
        mode,
        source_lang: config.languages.source.clone(),
        target_lang: config.languages.target.clone(),
        model_path: config.translation.model_path.clone(),
        gpu_layers: config.translation.n_gpu_layers,
        selected_input,
        selected_output,
    }
}

#[tauri::command]
fn set_languages(state: State<AppState>, source: String, target: String) -> StatusResponse {
    {
        let mut config = state.config.lock().unwrap();
        config.languages.source = source;
        config.languages.target = target;
    }
    get_status(state)
}

#[tauri::command]
fn set_mode(state: State<AppState>, mode: String) -> StatusResponse {
    *state.current_mode.lock().unwrap() = mode;
    get_status(state)
}

#[tauri::command]
fn toggle_pipeline(state: State<AppState>) -> StatusResponse {
    let mut is_running = state.is_running.lock().unwrap();
    *is_running = !*is_running;
    let running = *is_running;
    let mode = state.current_mode.lock().unwrap().clone();
    info!(
        "Pipeline {}: mode={}",
        if running { "started" } else { "stopped" },
        mode
    );
    drop(is_running);
    get_status(state)
}

#[tauri::command]
fn list_audio_devices(state: State<AppState>) -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let mut devices = Vec::new();

    let default_in = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    let default_out = host
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let selected_in = state.selected_input.lock().unwrap().clone();
    let selected_out = state.selected_output.lock().unwrap().clone();

    if let Ok(input_devices) = host.input_devices() {
        for dev in input_devices {
            if let Ok(name) = dev.name() {
                let is_selected = if selected_in.is_empty() {
                    name == default_in
                } else {
                    name == selected_in
                };
                devices.push(AudioDevice {
                    is_default: name == default_in,
                    is_selected,
                    name,
                    is_input: true,
                });
            }
        }
    }

    if let Ok(output_devices) = host.output_devices() {
        for dev in output_devices {
            if let Ok(name) = dev.name() {
                let is_selected = if selected_out.is_empty() {
                    name == default_out
                } else {
                    name == selected_out
                };
                devices.push(AudioDevice {
                    is_default: name == default_out,
                    is_selected,
                    name,
                    is_input: false,
                });
            }
        }
    }

    devices
}

#[tauri::command]
fn select_input_device(state: State<AppState>, name: String, app: tauri::AppHandle) -> Vec<AudioDevice> {
    info!("Selected input device: {}", name);
    *state.selected_input.lock().unwrap() = name.clone();

    // Restart monitoring on new device
    start_mic_monitor(&state, &app);

    list_audio_devices(state)
}

#[tauri::command]
fn select_output_device(state: State<AppState>, name: String) -> Vec<AudioDevice> {
    info!("Selected output device: {}", name);
    *state.selected_output.lock().unwrap() = name;
    list_audio_devices(state)
}

#[tauri::command]
fn get_mic_level(state: State<AppState>) -> f32 {
    let monitor = state.monitor.lock().unwrap();
    monitor.peak_level.load(Ordering::Relaxed) as f32 / 1000.0
}

#[tauri::command]
fn start_monitoring(state: State<AppState>, app: tauri::AppHandle) {
    start_mic_monitor(&state, &app);
}

#[tauri::command]
fn stop_monitoring(state: State<AppState>) {
    state.monitoring_active.store(false, Ordering::Relaxed);
    let mut monitor = state.monitor.lock().unwrap();
    monitor.stream = SendStream(None);
    monitor.peak_level.store(0, Ordering::Relaxed);
    info!("Mic monitoring stopped");
}

fn start_mic_monitor(state: &State<AppState>, app: &tauri::AppHandle) {
    let host = cpal::default_host();

    let selected = state.selected_input.lock().unwrap().clone();

    // Find the device
    let device = if selected.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()
            .ok()
            .and_then(|mut devs| devs.find(|d| d.name().ok().as_deref() == Some(&selected)))
    };

    let device = match device {
        Some(d) => d,
        None => {
            tracing::error!("No input device found for: {}", selected);
            return;
        }
    };

    let device_name = device.name().unwrap_or_else(|_| "unknown".into());
    info!("Starting mic monitor on: {}", device_name);

    // Stop existing monitor
    {
        let mut monitor = state.monitor.lock().unwrap();
        monitor.stream = SendStream(None);
    }

    let peak = state.monitor.lock().unwrap().peak_level.clone();
    let active = state.monitoring_active.clone();
    active.store(true, Ordering::Relaxed);

    let app_handle = app.clone();

    // Use the device's default config instead of forcing mono 16kHz
    let default_config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to get default config for '{}': {}", device_name, e);
            return;
        }
    };

    let channels = default_config.channels();
    let sample_format = default_config.sample_format();
    let config: cpal::StreamConfig = default_config.into();

    info!(
        "Mic monitor config: {}Hz, {} ch, {:?}",
        config.sample_rate.0, channels, sample_format
    );

    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            let peak = peak.clone();
            let active = active.clone();
            let app_handle = app_handle.clone();
            let ch = channels as usize;
            device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !active.load(Ordering::Relaxed) || data.is_empty() {
                        return;
                    }
                    // RMS across all channels (use first channel if multi-channel)
                    let step = ch.max(1);
                    let rms = (data.iter().step_by(step).map(|s| s * s).sum::<f32>()
                        / (data.len() / step) as f32)
                        .sqrt();
                    let level = (rms * 5000.0).min(1000.0) as u32;
                    peak.store(level, Ordering::Relaxed);
                    let _ = app_handle.emit("mic-level", level as f32 / 1000.0);
                },
                |err| tracing::error!("Mic monitor error: {}", err),
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let peak = peak.clone();
            let active = active.clone();
            let app_handle = app_handle.clone();
            let ch = channels as usize;
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if !active.load(Ordering::Relaxed) || data.is_empty() {
                        return;
                    }
                    let step = ch.max(1);
                    let rms = (data.iter().step_by(step)
                        .map(|&s| { let f = s as f32 / i16::MAX as f32; f * f })
                        .sum::<f32>()
                        / (data.len() / step) as f32)
                        .sqrt();
                    let level = (rms * 5000.0).min(1000.0) as u32;
                    peak.store(level, Ordering::Relaxed);
                    let _ = app_handle.emit("mic-level", level as f32 / 1000.0);
                },
                |err| tracing::error!("Mic monitor error: {}", err),
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let peak = peak.clone();
            let active = active.clone();
            let app_handle = app_handle.clone();
            let ch = channels as usize;
            device.build_input_stream(
                &config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if !active.load(Ordering::Relaxed) || data.is_empty() {
                        return;
                    }
                    let step = ch.max(1);
                    let rms = (data.iter().step_by(step)
                        .map(|&s| { let f = (s as f32 / 32768.0) - 1.0; f * f })
                        .sum::<f32>()
                        / (data.len() / step) as f32)
                        .sqrt();
                    let level = (rms * 5000.0).min(1000.0) as u32;
                    peak.store(level, Ordering::Relaxed);
                    let _ = app_handle.emit("mic-level", level as f32 / 1000.0);
                },
                |err| tracing::error!("Mic monitor error: {}", err),
                None,
            )
        }
        _ => {
            tracing::error!("Unsupported sample format: {:?}", sample_format);
            return;
        }
    };

    match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                tracing::error!("Failed to play mic monitor stream: {}", e);
                return;
            }
            let mut monitor = state.monitor.lock().unwrap();
            monitor.stream = SendStream(Some(s));
            monitor.active_device = device_name.clone();
            info!("Mic monitor active on: {}", device_name);
        }
        Err(e) => {
            tracing::error!("Failed to build mic monitor stream on '{}': {}", device_name, e);
        }
    }
}

#[tauri::command]
fn test_translation(state: State<AppState>, request: TranslateRequest) -> Result<TranslateResponse, String> {
    let config = state.config.lock().unwrap();
    let engine = voice_core::translation::engine::TranslationEngine::new(&config.translation)
        .map_err(|e| format!("Failed to init engine: {e}"))?;

    let result = engine
        .translate(&request.text, &request.source_lang, &request.target_lang)
        .map_err(|e| format!("Translation failed: {e}"))?;

    Ok(TranslateResponse {
        translation: result,
    })
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = AppConfig::load_default().unwrap_or_else(|e| {
        eprintln!("Failed to load config: {e}");
        panic!("Config file required: config/default.toml");
    });

    // Determine default devices
    let host = cpal::default_host();
    let default_input = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    let default_output = host
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            config: Mutex::new(config),
            is_running: Mutex::new(false),
            current_mode: Mutex::new("passthrough".to_string()),
            selected_input: Mutex::new(default_input),
            selected_output: Mutex::new(default_output),
            monitor: Mutex::new(AudioMonitor {
                stream: SendStream(None),
                peak_level: Arc::new(AtomicU32::new(0)),
                active_device: String::new(),
            }),
            monitoring_active: Arc::new(AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_languages,
            set_mode,
            toggle_pipeline,
            list_audio_devices,
            select_input_device,
            select_output_device,
            get_mic_level,
            start_monitoring,
            stop_monitoring,
            test_translation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
