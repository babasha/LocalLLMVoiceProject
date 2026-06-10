@echo off
REM Internal launcher used by ui_win.ps1 — runs the translator in UI mode and
REM redirects its clean output stream / logs to temp files the GUI tails.
set "PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin\x64;C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin;%~dp0libs\sherpa-onnx-win;%PATH%"
set "VOICE_UI=1"
set "RUST_LOG=warn"
REM VOICE_TTS_OUTPUT / VOICE_TTS_SPEAK / VOICE_TTS_ENGINE are inherited from the
REM GUI (ui_win.ps1) so the output device can be chosen in the window.
cd /d "%~dp0"
"C:\cargo-target\voice-translator\release\voice-translator.exe" --mode full 1> "%TEMP%\voice_ui_stream.txt" 2> "%TEMP%\voice_ui_err.txt"
