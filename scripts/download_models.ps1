# Download all models for voice-translator
# Requires: PowerShell 5.1+

$ModelsDir = Join-Path $PSScriptRoot ".." "models"
New-Item -ItemType Directory -Force -Path $ModelsDir | Out-Null

Write-Host "=== Downloading models to $ModelsDir ===" -ForegroundColor Cyan

# Whisper large-v3-turbo Q8
$WhisperUrl = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q8_0.bin"
$WhisperPath = Join-Path $ModelsDir "ggml-large-v3-turbo-q8.bin"
if (-not (Test-Path $WhisperPath)) {
    Write-Host "Downloading Whisper large-v3-turbo Q8..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri $WhisperUrl -OutFile $WhisperPath
    Write-Host "Done: $WhisperPath" -ForegroundColor Green
} else {
    Write-Host "Whisper model already exists, skipping." -ForegroundColor Gray
}

# Qwen3.5-2B Q4_K_M GGUF
$QwenUrl = "https://huggingface.co/AaryanK/Qwen3.5-2B-GGUF/resolve/main/Qwen3.5-2B-Q4_K_M.gguf"
$QwenPath = Join-Path $ModelsDir "Qwen3.5-2B-Q4_K_M.gguf"
if (-not (Test-Path $QwenPath)) {
    Write-Host "Downloading Qwen3.5-2B Q4_K_M GGUF..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri $QwenUrl -OutFile $QwenPath
    Write-Host "Done: $QwenPath" -ForegroundColor Green
} else {
    Write-Host "Qwen model already exists, skipping." -ForegroundColor Gray
}

# Kokoro TTS model (placeholder URL - check kokorox GitHub for actual URL)
Write-Host ""
Write-Host "NOTE: Kokoro and Piper TTS models need to be downloaded manually:" -ForegroundColor Yellow
Write-Host "  Kokoro: https://github.com/WismutHansen/kokorox/releases" -ForegroundColor White
Write-Host "  Piper:  https://github.com/rhasspy/piper/blob/master/VOICES.md" -ForegroundColor White

Write-Host ""
Write-Host "=== Model download complete ===" -ForegroundColor Cyan
