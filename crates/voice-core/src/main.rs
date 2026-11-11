use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use tracing::info;
use tracing_subscriber::EnvFilter;

use voice_core::audio;
use voice_core::config::AppConfig;
use voice_core::pipeline::orchestrator::{Orchestrator, PipelineMode};

#[derive(Debug, Clone, ValueEnum)]
enum Mode {
    /// Audio passthrough: mic → speaker (test audio setup)
    Passthrough,
    /// VAD + STT only: mic → transcribed text in console
    Stt,
    /// You speak: mic → VAD → STT → translation → TTS
    Full,
    /// The other party: system audio (loopback) → STT → translated subtitles
    Listen,
    /// Two-way: `full` and `listen` at the same time
    Duplex,
}

#[derive(Parser, Debug)]
#[command(name = "voice-translator")]
#[command(about = "Real-time local voice translator with native GPU acceleration")]
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

    /// With --list-devices: machine-readable `IN|OUT[*]\t<name>` lines
    #[arg(long)]
    tsv: bool,

    /// Source language override (what you speak)
    #[arg(long)]
    source_lang: Option<String>,

    /// Target language override (what the listener hears)
    #[arg(long)]
    target_lang: Option<String>,

    /// Microphone name substring ("" = default)
    #[arg(long)]
    input_device: Option<String>,

    /// Output device to capture in listen/duplex mode ("" = default output)
    #[arg(long)]
    loopback_device: Option<String>,

    /// Output device for the spoken translation ("" = default speakers)
    #[arg(long)]
    tts_output: Option<String>,

    /// Speak the translation aloud (true/false)
    #[arg(long)]
    speak: Option<bool>,

    /// Translation LLM (GGUF path)
    #[arg(long)]
    llm_model: Option<String>,

    /// VAD end-of-phrase silence in ms
    #[arg(long)]
    silence_ms: Option<u32>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    if cli.list_devices {
        if cli.tsv {
            audio::device::print_devices_tsv()?;
        } else {
            audio::device::print_all_devices()?;
        }
        return Ok(());
    }

    let mut config = AppConfig::load(&cli.config)?;

    if let Some(v) = cli.source_lang { config.languages.source = v; }
    if let Some(v) = cli.target_lang { config.languages.target = v; }
    if let Some(v) = cli.input_device { config.audio.input_device = v; }
    if let Some(v) = cli.loopback_device { config.audio.loopback_device = v; }
    if let Some(v) = cli.tts_output { config.tts.output_device = v; }
    if let Some(v) = cli.speak { config.tts.speak = v; }
    if let Some(v) = cli.llm_model { config.translation.model_path = v; }
    if let Some(v) = cli.silence_ms { config.vad.silence_duration_ms = v; }

    info!("Voice Translator starting");
    info!("Languages: {} → {}", config.languages.source, config.languages.target);
    info!("LLM: {}", config.translation.model_path);

    let pipeline_mode = match cli.mode {
        Mode::Passthrough => PipelineMode::Passthrough,
        Mode::Stt => PipelineMode::SttOnly,
        Mode::Full => PipelineMode::Full,
        Mode::Listen => PipelineMode::Listen,
        Mode::Duplex => PipelineMode::Duplex,
    };

    let orchestrator = Orchestrator::new(config, pipeline_mode);
    orchestrator.run()?;

    Ok(())
}
