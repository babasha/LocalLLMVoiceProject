use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::DeviceTrait;
use crossbeam_channel::{bounded, Receiver};
use rtrb::RingBuffer;
use tracing::{debug, error, info, warn};

use crate::audio::{capture, device, playback};
use crate::config::{AppConfig, SttConfig, TtsConfig};
use crate::error::{Result, VoiceTranslatorError};
use crate::pipeline::shutdown::ShutdownSignal;
use crate::stt::SpeechRecognizer;
use crate::translation::engine::TranslationEngine;
use crate::tts::EchoGate;
use crate::vad::detector::VadDetector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineMode {
    Passthrough,
    SttOnly,
    /// You speak → mic → translated → spoken (the original pipeline).
    Full,
    /// The other party (system audio via loopback) → translated subtitles.
    Listen,
    /// Both at once: two-way conversation translator.
    Duplex,
}

pub struct Orchestrator {
    config: AppConfig,
    mode: PipelineMode,
    shutdown: ShutdownSignal,
}

/// Everything that differs between the "me" (mic) and "them" (loopback) paths.
struct ChannelSpec {
    /// Short id used in UI records: "me" | "them".
    id: &'static str,
    device: cpal::Device,
    stt: SttConfig,
    src_lang: String,
    tgt_lang: String,
    /// Speak the translation (only ever for "me").
    tts: Option<TtsConfig>,
    /// While active, captured audio is replaced by silence (echo suppression).
    mute_gate: Option<Arc<EchoGate>>,
    /// Our TTS extends this while it plays.
    speak_gate: Option<Arc<EchoGate>>,
}

/// The translation-side part of a channel (no audio handles: those stay on
/// the main thread).
struct TrSpec {
    id: &'static str,
    src_lang: String,
    tgt_lang: String,
    tts: Option<TtsConfig>,
    speak_gate: Option<Arc<EchoGate>>,
}

/// Running capture + STT for one channel; translation is attached once the
/// LLM has finished loading.
struct ChannelRuntime {
    stream: cpal::Stream,
    rx: Receiver<SttEvent>,
    spec: TrSpec,
}

impl Orchestrator {
    pub fn new(config: AppConfig, mode: PipelineMode) -> Self {
        let shutdown = ShutdownSignal::new();
        Orchestrator { config, mode, shutdown }
    }

    pub fn run(&self) -> Result<()> {
        match self.mode {
            PipelineMode::Passthrough => self.run_passthrough(),
            PipelineMode::SttOnly => self.run_stt_only(),
            PipelineMode::Full | PipelineMode::Listen | PipelineMode::Duplex => self.run_translate(),
        }
    }

    fn run_passthrough(&self) -> Result<()> {
        info!("Starting passthrough mode (mic → speaker)");
        let sr = self.config.audio.sample_rate;
        let ch = self.config.audio.channels;
        let cap = self.config.audio.ring_buffer_capacity;

        let (prod, cons) = RingBuffer::<f32>::new(cap);
        let in_dev = device::microphone(&self.config.audio.input_device)?;
        let out_dev = device::default_output_device()?;
        info!("Input:  {}", in_dev.name().unwrap_or_default());
        info!("Output: {}", out_dev.name().unwrap_or_default());

        let _in_stream = capture::start_capture(&in_dev, sr, ch, prod)?;
        let _out_stream = playback::start_playback(&out_dev, sr, ch, cons)?;

        info!("Passthrough active. Press Ctrl+C to stop.");
        self.wait_for_shutdown();
        info!("Passthrough stopped.");
        Ok(())
    }

    fn run_stt_only(&self) -> Result<()> {
        info!("Starting streaming STT mode");
        let sample_rate = self.config.audio.sample_rate;

        let (producer, mut consumer) = RingBuffer::<f32>::new(sample_rate as usize * 8);
        let input_device = device::microphone(&self.config.audio.input_device)?;
        let _input_stream = capture::start_capture_converted(&input_device, sample_rate, producer)?;

        let shutdown = self.shutdown.clone();
        let stt_cfg = self.config.stt.clone();
        let vad_cfg = self.config.vad.clone();

        let _handle = thread::Builder::new().name("stt".into()).spawn(move || {
            let stt = match SpeechRecognizer::new(&stt_cfg) {
                Ok(s) => s, Err(e) => { error!("STT: {e}"); return; }
            };
            let vad = match VadDetector::new(&vad_cfg, sample_rate) {
                Ok(v) => v, Err(e) => { error!("VAD: {e}"); return; }
            };
            info!("STT + Silero VAD ready");
            println!("--- говорите ---");

            let partial_every = sample_rate as usize;
            let chunk = 512usize;
            let mut tmp = vec![0.0f32; chunk];
            let mut pbuf: Vec<f32> = Vec::new();
            let mut since_partial = 0usize;

            while !shutdown.is_shutdown() {
                let avail = consumer.slots();
                if avail < chunk { thread::sleep(Duration::from_millis(5)); continue; }
                let n = avail.min(chunk);
                for i in 0..n { tmp[i] = consumer.pop().unwrap_or(0.0); }
                let frame = &tmp[..n];
                vad.accept_waveform(frame);

                while let Some(seg) = vad.pop_segment() {
                    if let Ok(t) = stt.transcribe(&seg) {
                        if !t.is_empty() { println!("\r\x1B[2K{t}"); }
                        else { print!("\r\x1B[2K"); std::io::stdout().flush().ok(); }
                    }
                    pbuf.clear(); since_partial = 0;
                }

                if vad.is_speech() {
                    pbuf.extend_from_slice(frame);
                    since_partial += n;
                    if since_partial >= partial_every && pbuf.len() > sample_rate as usize / 2 {
                        if let Ok(t) = stt.transcribe(&pbuf) {
                            if !t.is_empty() { print!("\r\x1B[2K{t}"); std::io::stdout().flush().ok(); }
                        }
                        since_partial = 0;
                    }
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("STT thread: {e}")))?;

        info!("STT active. Ctrl+C to stop.");
        self.wait_for_shutdown();
        Ok(())
    }

    /// Build the channel list for the current mode.
    fn channel_specs(&self) -> Result<Vec<ChannelSpec>> {
        let cfg = &self.config;
        let want_me = matches!(self.mode, PipelineMode::Full | PipelineMode::Duplex);
        let want_them = matches!(self.mode, PipelineMode::Listen | PipelineMode::Duplex);

        // Echo suppression is needed only when our voice goes out through the
        // very device we loopback-capture (e.g. both are the default speakers).
        let gate = (want_me && want_them && cfg.tts.speak
            && device::same_output(&cfg.tts.output_device, &cfg.audio.loopback_device))
            .then(|| {
                warn!(
                    "TTS plays through the loopback-captured device: the other party's \
                     channel is muted while the translation is spoken. Use headphones \
                     or a virtual cable output to avoid this."
                );
                EchoGate::new()
            });

        let mut specs = Vec::new();
        if want_me {
            let dev = device::microphone(&cfg.audio.input_device)?;
            info!("[me]   capture: {}", dev.name().unwrap_or_default());
            specs.push(ChannelSpec {
                id: "me",
                device: dev,
                stt: cfg.stt.clone(),
                src_lang: cfg.languages.source.clone(),
                tgt_lang: cfg.languages.target.clone(),
                tts: cfg.tts.speak.then(|| cfg.tts.clone()),
                mute_gate: None,
                speak_gate: gate.clone(),
            });
        }
        if want_them {
            let stt = cfg.stt_listen.clone().ok_or_else(|| {
                VoiceTranslatorError::Config(
                    "listen/duplex mode needs an [stt_listen] section (e.g. Parakeet v3) — \
                     download it in the UI's Models tab"
                        .into(),
                )
            })?;
            let dev = device::loopback_source(&cfg.audio.loopback_device)?;
            info!("[them] loopback capture: {}", dev.name().unwrap_or_default());
            specs.push(ChannelSpec {
                id: "them",
                device: dev,
                stt,
                src_lang: cfg.languages.target.clone(),
                tgt_lang: cfg.languages.source.clone(),
                tts: None,
                mute_gate: gate,
                speak_gate: None,
            });
        }
        Ok(specs)
    }

    /// Translation pipeline for one or two channels:
    /// capture → Silero VAD → STT (LocalAgreement-2 partials) → LLM → [TTS].
    fn run_translate(&self) -> Result<()> {
        info!("Starting translation pipeline ({:?})", self.mode);
        let specs = self.channel_specs()?;
        let ui = Arc::new(Mutex::new(Ui::new(specs.len() > 1)));

        // Load the LLM in the background while capture + STT come up.
        let tr_cfg = self.config.translation.clone();
        let llm_loader = thread::Builder::new()
            .name("llm-load".into())
            .spawn(move || TranslationEngine::new(&tr_cfg).map(Arc::new))
            .map_err(|e| VoiceTranslatorError::Pipeline(format!("LLM loader: {e}")))?;

        let mut runtimes = Vec::new();
        for spec in specs {
            runtimes.push(self.start_capture_and_stt(spec, ui.clone())?);
        }

        let engine = llm_loader
            .join()
            .map_err(|_| VoiceTranslatorError::Pipeline("LLM loader panicked".into()))??;
        info!("Translation engine ready");

        // One GPU, one model: serialize LLM calls between the two channels.
        let llm_lock = Arc::new(Mutex::new(()));
        let mut streams = Vec::new();
        for rt in runtimes {
            let ChannelRuntime { stream, rx, spec } = rt;
            self.spawn_translation(spec, rx, engine.clone(), llm_lock.clone(), ui.clone())?;
            streams.push(stream);
        }
        ui.lock().unwrap().ready();

        info!("Pipeline active. Ctrl+C to stop.");
        self.wait_for_shutdown();
        info!("Pipeline stopped.");
        drop(streams);
        Ok(())
    }

    fn start_capture_and_stt(&self, spec: ChannelSpec, ui: Arc<Mutex<Ui>>) -> Result<ChannelRuntime> {
        let sample_rate = self.config.audio.sample_rate;
        let (producer, mut consumer) = RingBuffer::<f32>::new(sample_rate as usize * 8);
        let stream = capture::start_capture_converted(&spec.device, sample_rate, producer)?;

        let (tx, rx) = bounded::<SttEvent>(32);
        let shutdown = self.shutdown.clone();
        let stt_cfg = spec.stt.clone();
        let vad_cfg = self.config.vad.clone();
        let chan = spec.id;
        let src_tag = spec.src_lang.to_uppercase();
        let mute = spec.mute_gate.clone();

        thread::Builder::new().name(format!("stt-{chan}")).spawn(move || {
            let stt = match SpeechRecognizer::new(&stt_cfg) {
                Ok(s) => s, Err(e) => { error!("[{chan}] STT: {e}"); ui.lock().unwrap().warn(chan, &format!("STT: {e}")); return; }
            };
            let vad = match VadDetector::new(&vad_cfg, sample_rate) {
                Ok(v) => v, Err(e) => { error!("[{chan}] VAD: {e}"); return; }
            };
            info!("[{chan}] STT + Silero VAD ready ({})", stt.label());

            let perf = std::env::var("VOICE_PERF").is_ok();
            // Chunk-based streaming ASR with LocalAgreement-2 (Whisper-Streaming
            // style): re-transcribe the growing utterance buffer ~every 0.4s and
            // commit only the prefix that agreed between the two most recent
            // transcriptions. Stable words are streamed to translation before
            // the speaker pauses; VAD still bounds/segments the utterance.
            let partial_every = sample_rate as usize * 2 / 5;
            let chunk = 512usize;
            let mut tmp = vec![0.0f32; chunk];
            let mut pbuf: Vec<f32> = Vec::new();
            let mut since_partial = 0usize;
            let mut prev_text = String::new();
            let mut committed = 0usize;
            let mut muted = false;
            // Silero's max_speech_duration only *lowers the pause needed* to
            // split; continuous speech (a video, a monologue over music) can
            // still grow one segment indefinitely — and with it the cost of
            // every partial re-transcription and the LLM prompt. Hard limit:
            let hard_limit = sample_rate as usize * (vad_cfg.max_speech_duration_ms as usize + 1000) / 1000;

            // WASAPI loopback delivers *no packets at all* while nothing plays,
            // so VAD would never see the trailing silence and the utterance
            // would never be finalized. After STARVE of no data, synthesize
            // silence at real-time rate. STARVE is well above any normal
            // device buffer period, so a live mic is never padded.
            const STARVE: Duration = Duration::from_millis(200);
            let chunk_dur = Duration::from_secs_f64(chunk as f64 / sample_rate as f64);
            let mut last_data = Instant::now();
            // Silero needs a few frames (+ min_speech_duration) before it
            // reports speech, so the partial buffer would miss the first word
            // or two ("Ask not what…" → "what…"). Keep the recent audio and
            // prepend it when speech starts, like VAD segments' own padding.
            let preroll_len = sample_rate as usize * 6 / 10;
            let mut preroll: std::collections::VecDeque<f32> = std::collections::VecDeque::with_capacity(preroll_len + chunk);

            while !shutdown.is_shutdown() {
                let avail = consumer.slots();
                let n = if avail >= chunk {
                    for i in 0..chunk { tmp[i] = consumer.pop().unwrap_or(0.0); }
                    last_data = Instant::now();
                    chunk
                } else if last_data.elapsed() >= STARVE + chunk_dur {
                    tmp.fill(0.0);
                    last_data += chunk_dur;
                    chunk
                } else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };

                // Echo suppression: while our own TTS is audible on the
                // captured device, feed silence so VAD closes the segment and
                // nothing of our voice is transcribed.
                let now_muted = mute.as_ref().is_some_and(|g| g.active());
                if now_muted {
                    tmp[..n].fill(0.0);
                    if !muted { pbuf.clear(); prev_text.clear(); committed = 0; }
                }
                muted = now_muted;

                let frame = &tmp[..n];
                vad.accept_waveform(frame);

                while let Some(seg) = vad.pop_segment() {
                    let t_stt = perf.then(Instant::now);
                    let tr = stt.transcribe(&seg);
                    if let Some(t) = t_stt {
                        let audio_s = seg.len() as f64 / sample_rate as f64;
                        let stt_ms = t.elapsed().as_secs_f64() * 1000.0;
                        let rtf = if audio_s > 0.0 { (stt_ms / 1000.0) / audio_s } else { 0.0 };
                        eprintln!("[PERF] [{chan}] STT: {stt_ms:.0}ms for {audio_s:.2}s audio (RTF {rtf:.2})");
                    }
                    if let Ok(text) = tr {
                        if !text.is_empty() {
                            ui.lock().unwrap().source(chan, &src_tag, &text);
                            // Blocking send is fine: the translation thread
                            // drains and merges backlog, so it never stalls long.
                            let _ = tx.send(SttEvent::Final { text, at: Instant::now() });
                        }
                    }
                    pbuf.clear();
                    since_partial = 0;
                    prev_text.clear();
                    committed = 0;
                }

                let speaking = vad.is_speech() && !now_muted;
                if speaking && pbuf.is_empty() {
                    pbuf.extend(preroll.iter());
                }
                preroll.extend(frame.iter());
                if preroll.len() > preroll_len {
                    preroll.drain(..preroll.len() - preroll_len);
                }

                if speaking {
                    pbuf.extend_from_slice(frame);
                    if pbuf.len() >= hard_limit {
                        // Close the segment now; it is popped and finalized on
                        // the next iteration, and VAD starts a fresh one.
                        debug!("[{chan}] force-flushing {:.1}s segment", pbuf.len() as f32 / sample_rate as f32);
                        vad.flush();
                        pbuf.clear();
                        since_partial = 0;
                        continue;
                    }
                    since_partial += n;
                    if since_partial >= partial_every && pbuf.len() > sample_rate as usize / 2 {
                        if let Ok(cur) = stt.transcribe(&pbuf) {
                            if !cur.is_empty() {
                                ui.lock().unwrap().source(chan, &src_tag, &cur);
                                let agreed = common_word_prefix(&prev_text, &cur);
                                if agreed > committed {
                                    committed = agreed;
                                    let stable: String = cur
                                        .split_whitespace()
                                        .take(agreed)
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    let _ = tx.try_send(SttEvent::Partial { text: stable, at: Instant::now() });
                                }
                                prev_text = cur;
                            }
                        }
                        since_partial = 0;
                    }
                }
            }
            vad.flush();
            while let Some(seg) = vad.pop_segment() {
                if let Ok(text) = stt.transcribe(&seg) {
                    if !text.is_empty() { let _ = tx.send(SttEvent::Final { text, at: Instant::now() }); }
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("STT thread: {e}")))?;

        let tr = TrSpec {
            id: spec.id,
            src_lang: spec.src_lang,
            tgt_lang: spec.tgt_lang,
            tts: spec.tts,
            speak_gate: spec.speak_gate,
        };
        Ok(ChannelRuntime { stream, rx, spec: tr })
    }

    fn spawn_translation(
        &self,
        spec: TrSpec,
        rx: Receiver<SttEvent>,
        engine: Arc<TranslationEngine>,
        llm_lock: Arc<Mutex<()>>,
        ui: Arc<Mutex<Ui>>,
    ) -> Result<()> {
        let shutdown = self.shutdown.clone();
        let pcfg = self.config.pipeline.clone();
        let chan = spec.id;

        thread::Builder::new().name(format!("translate-{chan}")).spawn(move || {
            let mut ctx = match engine.make_context() {
                Ok(c) => c, Err(e) => { error!("[{chan}] Context: {e}"); return; }
            };
            let tag = spec.tgt_lang.to_uppercase();
            let speaker = spec.tts.as_ref().and_then(|cfg| {
                match crate::tts::create_speaker(cfg, spec.speak_gate.clone()) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        error!("[{chan}] TTS disabled: {e}");
                        ui.lock().unwrap().warn(chan, &format!("TTS disabled: {e}"));
                        None
                    }
                }
            });
            let speak = |s: &str| if let Some(sp) = &speaker { if !s.is_empty() { sp.speak(s) } };
            // Serialized against the other channel (one model on one GPU).
            macro_rules! translate {
                ($text:expr, $prefix:expr) => {{
                    let text: &str = $text;
                    if crate::translation::prompt::is_already_target(text, &spec.tgt_lang) {
                        // Already in the target language: show as-is, no LLM call.
                        Ok(text.to_string())
                    } else {
                        let _g = llm_lock.lock().unwrap();
                        engine.translate_prefixed(&mut ctx, text, $prefix, &spec.src_lang, &spec.tgt_lang)
                    }
                }};
            }

            // Incremental ("simultaneous") streaming state, per utterance:
            //   spoken — the target-language text already committed (voiced +
            //   shown as "committed"). It is fed back to the model as a forced
            //   prefix so each new partial *continues* it instead of
            //   re-translating from scratch; the model therefore can never
            //   contradict words the listener already heard. The last HOLD
            //   generated words stay "draft": they may still change.
            const HOLD: usize = 2;
            let mut spoken = String::new();
            let mut lag_warned = false;

            while !shutdown.is_shutdown() {
                let ev = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(e) => e,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let mut events = vec![ev];
                while let Ok(newer) = rx.try_recv() { events.push(newer); }
                let batch = Batch::from_events(events);

                if !batch.finals.is_empty() {
                    let lag = batch.finals[0].1.elapsed();
                    let queued = batch.finals.len();
                    if lag.as_millis() as u64 > pcfg.lag_warn_ms || queued > 1 {
                        let msg = format!("lag {:.1}s, {queued} utterance(s) queued", lag.as_secs_f32());
                        warn!("[{chan}] translation {msg}");
                        ui.lock().unwrap().warn(chan, &msg);
                        lag_warned = true;
                    } else if lag_warned {
                        ui.lock().unwrap().warn(chan, "");
                        lag_warned = false;
                    }

                    // Too late to be useful in a live conversation: show the
                    // source untranslated and move on instead of falling further behind.
                    if pcfg.drop_after_ms > 0 && lag.as_millis() as u64 > pcfg.drop_after_ms {
                        warn!("[{chan}] dropping {queued} stale utterance(s) ({:.1}s old)", lag.as_secs_f32());
                        for (text, _) in &batch.finals {
                            ui.lock().unwrap().final_(chan, text, "");
                        }
                        spoken.clear();
                    } else {
                        let groups: Vec<String> = if pcfg.merge_backlog {
                            vec![batch.finals.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" ")]
                        } else {
                            batch.finals.iter().map(|(t, _)| t.clone()).collect()
                        };
                        for text in groups {
                            debug!("[{chan}] translating final: '{text}'");
                            // The prefix lock exists only so we never contradict
                            // words already *heard*. With no voice, re-translate
                            // the final from scratch: the full sentence gives the
                            // best translation, and a partial that missed or
                            // misheard the start cannot poison the result.
                            if speaker.is_none() {
                                spoken.clear();
                            }
                            match translate!(&text, &spoken) {
                                Ok(tr) if !tr.is_empty() => {
                                    let already = spoken.split_whitespace().count();
                                    let words: Vec<&str> = tr.split_whitespace().collect();
                                    if words.len() > already {
                                        speak(&words[already..].join(" "));
                                    }
                                    ui.lock().unwrap().final_(chan, &text, &tr);
                                }
                                Ok(_) => ui.lock().unwrap().final_(chan, &text, ""),
                                Err(e) => {
                                    error!("[{chan}] translate: {e}");
                                    ui.lock().unwrap().final_(chan, &text, "");
                                }
                            }
                            spoken.clear(); // only the first group continues the old prefix
                        }
                    }
                    // Fall through: a partial that arrived *after* these finals
                    // is the start of the next utterance — handle it right away.
                }

                if let Some((text, at)) = batch.latest_partial {
                    // A stale partial is not worth an LLM call — a fresher one
                    // or the final for this utterance is already on its way.
                    if at.elapsed().as_millis() as u64 > pcfg.lag_warn_ms {
                        continue;
                    }
                    if let Ok(tr) = translate!(&text, &spoken) {
                        if !tr.is_empty() {
                            let already = spoken.split_whitespace().count();
                            let words: Vec<&str> = tr.split_whitespace().collect();
                            let upto = words.len().saturating_sub(HOLD).max(already);
                            if upto > already {
                                speak(&words[already..upto].join(" "));
                                spoken = words[..upto].join(" ");
                            }
                            let draft = words[upto.min(words.len())..].join(" ");
                            ui.lock().unwrap().translation(chan, &tag, &spoken, &draft);
                        }
                    }
                }
            }
        }).map_err(|e| VoiceTranslatorError::Pipeline(format!("Translation thread: {e}")))?;
        Ok(())
    }

    fn wait_for_shutdown(&self) {
        let shutdown = self.shutdown.clone();
        let _ = ctrlc::set_handler(move || {
            info!("Shutting down...");
            shutdown.shutdown();
        });
        while !self.shutdown.is_shutdown() {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

// ---- Types ----

enum SttEvent {
    Partial { text: String, at: Instant },
    Final { text: String, at: Instant },
}

/// What the translation thread does with one drained batch of events:
/// every final (in order — none may be lost), plus only the newest partial.
struct Batch {
    finals: Vec<(String, Instant)>,
    latest_partial: Option<(String, Instant)>,
}

impl Batch {
    fn from_events(events: Vec<SttEvent>) -> Self {
        let mut finals = Vec::new();
        let mut latest_partial = None;
        for e in events {
            match e {
                SttEvent::Final { text, at } if text.chars().count() > 1 => {
                    finals.push((text, at));
                    // A partial received before this final belongs to the
                    // same (now finished) utterance.
                    latest_partial = None;
                }
                SttEvent::Partial { text, at } if text.chars().count() > 2 => {
                    latest_partial = Some((text, at))
                }
                _ => {}
            }
        }
        Batch { finals, latest_partial }
    }
}

// ---- Output: console (one or two channels) or GUI record stream ----

enum Ui {
    /// Single channel in a terminal: two lines updated in place.
    Console(TwoLineDisplay),
    /// Two channels in a terminal: log committed utterances only.
    ConsoleLog,
    /// Machine-readable records for the desktop UI (VOICE_UI=1).
    Gui,
}

impl Ui {
    fn new(multi: bool) -> Self {
        if std::env::var("VOICE_UI").is_ok() {
            Ui::Gui
        } else if multi {
            Ui::ConsoleLog
        } else {
            Ui::Console(TwoLineDisplay::new())
        }
    }

    fn ready(&mut self) {
        if let Ui::Gui = self {
            emit_ui("READY", "", "", "");
        }
    }

    /// Live (partial or final) source-language text.
    fn source(&mut self, chan: &str, tag: &str, text: &str) {
        match self {
            Ui::Console(d) => d.update_src(&format!("[{tag}] {text}")),
            Ui::ConsoleLog => {}
            Ui::Gui => emit_ui("SRC", chan, text, ""),
        }
    }

    /// Live translation split into `committed` (already voiced, final) and
    /// `draft` (may still change).
    fn translation(&mut self, chan: &str, tag: &str, committed: &str, draft: &str) {
        match self {
            Ui::Console(d) => d.update_tr(tag, committed, draft),
            Ui::ConsoleLog => {}
            Ui::Gui => emit_ui("TR", chan, committed, draft),
        }
    }

    fn final_(&mut self, chan: &str, src: &str, tr: &str) {
        match self {
            Ui::Console(d) => d.commit(src, tr),
            Ui::ConsoleLog => {
                let who = if chan == "me" { "ME  " } else { "THEM" };
                println!("{who} {src}");
                if !tr.is_empty() {
                    println!("     → {tr}");
                }
            }
            Ui::Gui => emit_ui("FINAL", chan, src, tr),
        }
    }

    /// Status warning for a channel ("" clears it).
    fn warn(&mut self, chan: &str, msg: &str) {
        match self {
            Ui::Gui => emit_ui("WARN", chan, msg, ""),
            _ if !msg.is_empty() => eprintln!("[{chan}] ! {msg}"),
            _ => {}
        }
    }
}

// Always occupies exactly 2 terminal lines when active:
//   Line 1: [RU] ...
//   Line 2: [EN] committed <dim>draft</dim>
// Cursor is ALWAYS at line 2 after any render.
struct TwoLineDisplay {
    src: String,
    tr: String,
    active: bool,
}

impl TwoLineDisplay {
    fn new() -> Self {
        TwoLineDisplay { src: String::new(), tr: String::new(), active: false }
    }

    fn update_src(&mut self, text: &str) {
        self.src = text.to_string();
        self.render();
    }

    fn update_tr(&mut self, tag: &str, committed: &str, draft: &str) {
        // Committed words in normal weight, draft dimmed (ANSI SGR 2).
        self.tr = if draft.is_empty() {
            format!("[{tag}] {committed}")
        } else {
            format!("[{tag}] {committed} \x1B[2m{draft}\x1B[0m")
        };
        self.render();
    }

    /// Print the finished utterance and reset for the next one.
    fn commit(&mut self, _src: &str, tr: &str) {
        let mut out = std::io::stdout();
        if self.active {
            write!(out, "\x1B[1A\r\x1B[2K").ok();
        }
        if !self.src.is_empty() {
            writeln!(out, "{}", self.src).ok();
        }
        write!(out, "\r\x1B[2K").ok();
        if !tr.is_empty() {
            // Keep the language tag from the live line, if any.
            let tag = self.tr.split(']').next().filter(|t| t.starts_with('[')).map(|t| format!("{t}] "));
            writeln!(out, "{}{tr}", tag.unwrap_or_default()).ok();
        }
        out.flush().ok();
        self.src.clear();
        self.tr.clear();
        self.active = false;
    }

    fn render(&mut self) {
        let mut out = std::io::stdout();
        if self.active {
            write!(out, "\x1B[1A").ok();
        }
        write!(out, "\r\x1B[2K{}", self.src).ok();
        write!(out, "\n\r\x1B[2K{}", self.tr).ok();
        out.flush().ok();
        self.active = true;
    }
}

/// Emit one tab-delimited record for the desktop UI, prefixed with US (0x1F)
/// so the GUI can ignore any other stdout noise:
///   READY                       — models loaded, listening
///   SRC   <chan> <text>         — live source text
///   TR    <chan> <committed> <draft>
///   FINAL <chan> <source> <translation>
///   WARN  <chan> <message>      — "" clears the warning
fn emit_ui(kind: &str, chan: &str, a: &str, b: &str) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut out = std::io::stdout();
    let _ = writeln!(out, "\u{1f}{kind}\t{chan}\t{}\t{}", clean(a), clean(b));
    let _ = out.flush();
}

/// Number of leading whitespace-separated words that `a` and `b` share.
/// Used for streaming "local agreement": the common prefix of two consecutive
/// partial transcriptions is considered stable.
fn common_word_prefix(a: &str, b: &str) -> usize {
    a.split_whitespace()
        .zip(b.split_whitespace())
        .take_while(|(x, y)| x == y)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: &str) -> SttEvent { SttEvent::Partial { text: t.into(), at: Instant::now() } }
    fn f(t: &str) -> SttEvent { SttEvent::Final { text: t.into(), at: Instant::now() } }

    #[test]
    fn prefix_agreement() {
        assert_eq!(common_word_prefix("я хочу пойти", "я хочу поехать домой"), 2);
        assert_eq!(common_word_prefix("", "привет"), 0);
    }

    #[test]
    fn batch_keeps_all_finals_in_order() {
        let b = Batch::from_events(vec![f("один раз"), p("два"), f("три раза"), f("четыре")]);
        let texts: Vec<_> = b.finals.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["один раз", "три раза", "четыре"]);
    }

    #[test]
    fn partial_before_final_is_discarded_partial_after_kept() {
        let b = Batch::from_events(vec![p("старое начало"), f("старое начало конец")]);
        assert!(b.latest_partial.is_none());
        let b = Batch::from_events(vec![f("фраза один"), p("новая фраза")]);
        assert_eq!(b.latest_partial.unwrap().0, "новая фраза");
    }

    #[test]
    fn newest_partial_wins() {
        let b = Batch::from_events(vec![p("раз два"), p("раз два три")]);
        assert_eq!(b.latest_partial.unwrap().0, "раз два три");
        assert!(b.finals.is_empty());
    }
}
