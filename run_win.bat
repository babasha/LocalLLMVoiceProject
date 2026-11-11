@echo off
setlocal
REM === Run the native Windows build in the console ===
REM Default: full pipeline (mic -> STT -> translation -> voice). Extra args pass
REM through, e.g.  run_win.bat --mode duplex   (a later --mode overrides the default).

REM Runtime DLLs: sherpa-onnx + onnxruntime (libs\sherpa-onnx-win) and the CUDA
REM runtime (cublas etc.; CUDA 13 keeps them in bin\x64).
if not defined CUDA_PATH set "CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
set "PATH=%CUDA_PATH%\bin\x64;%CUDA_PATH%\bin;%~dp0libs\sherpa-onnx-win;%PATH%"

set "EXE="
if defined CARGO_TARGET_DIR if exist "%CARGO_TARGET_DIR%\release\voice-translator.exe" set "EXE=%CARGO_TARGET_DIR%\release\voice-translator.exe"
if not defined EXE if exist "C:\cargo-target\voice-translator\release\voice-translator.exe" set "EXE=C:\cargo-target\voice-translator\release\voice-translator.exe"
if not defined EXE if exist "%~dp0target\release\voice-translator.exe" set "EXE=%~dp0target\release\voice-translator.exe"
if not defined EXE echo voice-translator.exe not found - run build_win.bat first & exit /b 1

cd /d "%~dp0"
"%EXE%" --mode full %*
