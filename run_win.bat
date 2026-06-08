@echo off
REM === Run the native Windows build ===
REM DLLs: sherpa-onnx + onnxruntime (libs\sherpa-onnx-win) and CUDA runtime (cublas etc.)
set "PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin\x64;C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin;%~dp0libs\sherpa-onnx-win;%PATH%"
cd /d "%~dp0"
REM Default to full pipeline (mic -> STT -> translate -> console); extra args pass through.
"C:\cargo-target\voice-translator\release\voice-translator.exe" --mode full %*
