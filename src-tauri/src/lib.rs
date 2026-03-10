use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::info;

use voice_core::config::AppConfig;

/// Application state shared between Tauri commands.
struct AppState {
    config: Mutex<AppConfig>,
    is_running: Mutex<bool>,
    current_mode: Mutex<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct StatusResponse {
    is_running: bool,
    mode: String,
    source_lang: String,
    target_lang: String,
    model_path: String,
    gpu_layers: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct AudioDevice {
    name: String,
    is_input: bool,
    is_default: bool,
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

    StatusResponse {
        is_running,
        mode,
        source_lang: config.languages.source.clone(),
        target_lang: config.languages.target.clone(),
        model_path: config.translation.model_path.clone(),
        gpu_layers: config.translation.n_gpu_layers,
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
fn list_audio_devices() -> Vec<AudioDevice> {
    use cpal::traits::{DeviceTrait, HostTrait};
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

    if let Ok(input_devices) = host.input_devices() {
        for dev in input_devices {
            if let Ok(name) = dev.name() {
                devices.push(AudioDevice {
                    is_default: name == default_in,
                    name,
                    is_input: true,
                });
            }
        }
    }

    if let Ok(output_devices) = host.output_devices() {
        for dev in output_devices {
            if let Ok(name) = dev.name() {
                devices.push(AudioDevice {
                    is_default: name == default_out,
                    name,
                    is_input: false,
                });
            }
        }
    }

    devices
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
        eprintln!("Failed to load config: {e}. Using defaults.");
        // Provide sensible defaults if config file is missing
        panic!("Config file required: config/default.toml");
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            config: Mutex::new(config),
            is_running: Mutex::new(false),
            current_mode: Mutex::new("passthrough".to_string()),
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            set_languages,
            set_mode,
            toggle_pipeline,
            list_audio_devices,
            test_translation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
