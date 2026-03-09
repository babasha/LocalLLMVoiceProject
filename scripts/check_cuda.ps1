# Check CUDA Toolkit installation and GPU availability
# Requires: NVIDIA GPU with CUDA support

Write-Host "=== CUDA Environment Check ===" -ForegroundColor Cyan

# Check NVIDIA driver
Write-Host "`nNVIDIA Driver:" -ForegroundColor Yellow
try {
    $smi = nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
    Write-Host "  $smi" -ForegroundColor Green
} catch {
    Write-Host "  nvidia-smi not found! Install NVIDIA drivers." -ForegroundColor Red
}

# Check CUDA Toolkit
Write-Host "`nCUDA Toolkit:" -ForegroundColor Yellow
if ($env:CUDA_PATH) {
    Write-Host "  CUDA_PATH: $($env:CUDA_PATH)" -ForegroundColor Green
    $nvcc = Join-Path $env:CUDA_PATH "bin" "nvcc.exe"
    if (Test-Path $nvcc) {
        $version = & $nvcc --version 2>&1 | Select-String "release"
        Write-Host "  $version" -ForegroundColor Green
    }
} else {
    Write-Host "  CUDA_PATH not set! Install CUDA Toolkit 12.2+" -ForegroundColor Red
}

# Check cuDNN
Write-Host "`ncuDNN:" -ForegroundColor Yellow
$cudnnDll = Join-Path $env:CUDA_PATH "bin" "cudnn64_*.dll" -ErrorAction SilentlyContinue
if ($cudnnDll -and (Test-Path $cudnnDll)) {
    Write-Host "  Found: $cudnnDll" -ForegroundColor Green
} else {
    Write-Host "  cuDNN DLL not found in CUDA_PATH/bin" -ForegroundColor Yellow
}

# Check MSVC
Write-Host "`nMSVC Build Tools:" -ForegroundColor Yellow
try {
    $cl = where.exe cl.exe 2>&1
    Write-Host "  cl.exe: $cl" -ForegroundColor Green
} catch {
    Write-Host "  cl.exe not found! Install Visual Studio Build Tools 2022" -ForegroundColor Red
}

# Check CMake
Write-Host "`nCMake:" -ForegroundColor Yellow
try {
    $cmake = cmake --version 2>&1 | Select-Object -First 1
    Write-Host "  $cmake" -ForegroundColor Green
} catch {
    Write-Host "  CMake not found! Install CMake 3.25+" -ForegroundColor Red
}

# Check Rust
Write-Host "`nRust:" -ForegroundColor Yellow
try {
    $rustc = rustc --version 2>&1
    Write-Host "  $rustc" -ForegroundColor Green
} catch {
    Write-Host "  Rust not found!" -ForegroundColor Red
}

Write-Host "`n=== Check complete ===" -ForegroundColor Cyan
