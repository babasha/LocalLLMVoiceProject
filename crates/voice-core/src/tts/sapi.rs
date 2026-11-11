//! Lightweight text-to-speech using the built-in Windows SAPI voice
//! (`System.Speech`). No model files, no GPU — just enough to *hear* the
//! live translation. A single PowerShell process is kept alive and fed lines
//! over stdin, so there is no per-utterance process-startup latency.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::info;

use crate::error::{Result, VoiceTranslatorError};
use crate::tts::{EchoGate, Speaker};

/// Speaks lines of text aloud through the Windows SAPI voice.
pub struct SapiSpeaker {
    _child: Child,
    stdin: Mutex<ChildStdin>,
    gate: Option<Arc<EchoGate>>,
    rate: i32,
}

impl SapiSpeaker {
    /// Start the speaker. `rate` is the SAPI rate (-10 slow .. 10 fast, 0 normal).
    pub fn new(rate: i32, gate: Option<Arc<EchoGate>>) -> Result<Self> {
        // Persistent reader loop: speak each non-empty line synchronously, so
        // utterances never overlap and stdin naturally queues backlog.
        let script = format!(
            "Add-Type -AssemblyName System.Speech; \
             [Console]::InputEncoding = [System.Text.Encoding]::UTF8; \
             $v = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $v.Rate = {rate}; \
             while (($line = [Console]::In.ReadLine()) -ne $null) {{ \
               if ($line.Trim().Length -gt 0) {{ $v.Speak($line) }} \
             }}"
        );

        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| VoiceTranslatorError::Tts(format!("Failed to start SAPI voice: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| VoiceTranslatorError::Tts("SAPI: no stdin handle".into()))?;

        info!("SAPI speaker ready (rate {rate})");
        Ok(SapiSpeaker {
            _child: child,
            stdin: Mutex::new(stdin),
            gate,
            rate,
        })
    }
}

impl Speaker for SapiSpeaker {
    /// Queue `text` to be spoken. Non-blocking from the caller's side; the
    /// PowerShell reader speaks it on its own thread. Errors are swallowed —
    /// a dead voice must never stall the translation pipeline.
    fn speak(&self, text: &str) {
        let line = text.replace(['\r', '\n'], " ");
        if let Some(g) = &self.gate {
            // SAPI gives no playback timing back; estimate ~2.5 words/s at
            // rate 0, scaled by the configured rate.
            let words = line.split_whitespace().count().max(1) as f32;
            let secs = words * 0.4 / (1.0 + self.rate as f32 * 0.1).max(0.3);
            g.extend(Duration::from_secs_f32(secs));
        }
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }
}
