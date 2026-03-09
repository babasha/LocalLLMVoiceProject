mod audio;
mod config;
mod error;
mod pipeline;
mod stt;
mod translation;
mod tts;
mod vad;

use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::pipeline::orchestrator::{Orchestrator, PipelineMode};

#[derive(Debug, Clone, ValueEnum)]
enum Mode {
    /// Audio passthrough: mic → speaker (test audio setup)
    Passthrough,
    /// VAD + STT only: mic → transcribed text in console
    Stt,
    /// Full pipeline: mic → VAD → STT → Translation → TTS → speaker
    Full,
}

#[derive(Parser, Debug)]
#[command(name = "voice-translator")]
#[command(about = "Real-time voice translator (RU↔EN) with native GPU acceleration")]
struct Cli {
    /// Pipeline mode
    #[arg(short, long, value_enum, default_value_t = Mode::Passthrough)]
    mode: Mode,

    /// Config file path
    #[arg(short, long, default_value = "config/default.toml")]
    config: PathBuf,

    /// List audio devices and exit
    #[arg(long)]
    list_devices: bool,

    /// Source language override
    #[arg(long)]
    source_lang: Option<String>,

    /// Target language override
    #[arg(long)]
    target_lang: Option<String>,
}

fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // List devices mode
    if cli.list_devices {
        audio::device::print_all_devices()?;
        return Ok(());
    }

    // Load config
    let mut config = AppConfig::load(&cli.config)?;

    // Apply CLI overrides
    if let Some(ref lang) = cli.source_lang {
        config.languages.source = lang.clone();
    }
    if let Some(ref lang) = cli.target_lang {
        config.languages.target = lang.clone();
    }

    info!("Voice Translator starting");
    info!(
        "Languages: {} → {}",
        config.languages.source, config.languages.target
    );
    info!("Audio: {}Hz, {} ch", config.audio.sample_rate, config.audio.channels);

    let pipeline_mode = match cli.mode {
        Mode::Passthrough => PipelineMode::Passthrough,
        Mode::Stt => PipelineMode::SttOnly,
        Mode::Full => PipelineMode::Full,
    };

    let orchestrator = Orchestrator::new(config, pipeline_mode);
    orchestrator.run()?;

    Ok(())
}
