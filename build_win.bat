@echo off
REM === Native Windows build for voice-core (MSVC + CUDA) ===
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set "CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
set "CUDA_PATH_V13_3=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
set "PATH=%CUDA_PATH%\bin;%USERPROFILE%\.cargo\bin;C:\Program Files\CMake\bin;%PATH%"
REM Use Ninja generator: drives nvcc directly, avoids MSBuild CUDA-targets fragility
set "CMAKE_GENERATOR=Ninja"
set "SHERPA_ONNX_LIB_DIR=C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject\libs\sherpa-onnx-win"
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
REM Build CUDA kernels only for this GPU (Blackwell sm_120) -> faster build, guaranteed support
set "CMAKE_CUDA_ARCHITECTURES=120"
REM Keep build artifacts OUT of the OneDrive-synced project tree
set "CARGO_TARGET_DIR=C:\cargo-target\voice-translator"
cd /d "C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject"

echo === ENV ===
where cl
where nvcc
where cmake
where cargo
echo SHERPA_ONNX_LIB_DIR=%SHERPA_ONNX_LIB_DIR%
echo CARGO_TARGET_DIR=%CARGO_TARGET_DIR%
echo CMAKE_CUDA_ARCHITECTURES=%CMAKE_CUDA_ARCHITECTURES%
echo === BUILD START ===
cargo build --release -p voice-core
echo === BUILD EXIT %ERRORLEVEL% ===
