# Check the Windows build prerequisites: NVIDIA driver, CUDA Toolkit, MSVC,
# CMake, LLVM and Rust.

function Report($name, [scriptblock]$probe, $hint) {
    Write-Host "`n${name}:" -ForegroundColor Yellow
    $out = $null
    try { $out = & $probe 2>$null | Select-Object -First 1 } catch {}
    if ($out) { Write-Host "  $out" -ForegroundColor Green }
    else { Write-Host "  not found - $hint" -ForegroundColor Red }
}

Write-Host "=== Build environment check ===" -ForegroundColor Cyan

Report "NVIDIA driver" { nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader } "install the NVIDIA driver"
Report "CUDA Toolkit" {
    if ($env:CUDA_PATH) { & (Join-Path $env:CUDA_PATH "bin\nvcc.exe") --version | Select-String "release" }
} "install CUDA Toolkit 12.8+ (winget install Nvidia.CUDA)"
Report "MSVC Build Tools" {
    & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
} "install VS 2022 Build Tools with the C++ workload"
Report "CMake" { cmake --version } "winget install Kitware.CMake"
Report "LLVM (libclang)" {
    $llvm = "C:\Program Files\LLVM\bin\libclang.dll"
    if (Test-Path $llvm) { $llvm } elseif ($env:LIBCLANG_PATH -and (Test-Path $env:LIBCLANG_PATH)) { $env:LIBCLANG_PATH }
} "winget install LLVM.LLVM"
Report "Rust" { rustc --version } "winget install Rustlang.Rustup"

Write-Host "`n=== Check complete ===" -ForegroundColor Cyan
