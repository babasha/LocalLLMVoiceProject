@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set "CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
set "CUDA_PATH_V13_3=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
set "PATH=%CUDA_PATH%\bin\x64;%CUDA_PATH%\bin;%USERPROFILE%\.cargo\bin;C:\Program Files\CMake\bin;%PATH%"
set "SHERPA_ONNX_LIB_DIR=C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject\libs\sherpa-onnx-win"
set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
set "CMAKE_GENERATOR=Ninja"
set "CMAKE_CUDA_ARCHITECTURES=120"
set "CARGO_TARGET_DIR=C:\cargo-target\voice-translator"
cd /d "C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject"

echo === UPDATE llama-cpp-2 -> 0.1.146 ===
cargo update -p llama-cpp-sys-2 --precise 0.1.146
cargo update -p llama-cpp-2 --precise 0.1.146
echo === BUILD (lib + example) ===
cargo build --release -p voice-core
cargo build --release -p voice-core --example test_llm
echo === BUILD EXIT %ERRORLEVEL% ===
REM refresh DLLs next to exes
copy /Y "%SHERPA_ONNX_LIB_DIR%\*.dll" "C:\cargo-target\voice-translator\release\" >nul
copy /Y "%SHERPA_ONNX_LIB_DIR%\*.dll" "C:\cargo-target\voice-translator\release\examples\" >nul
echo === DONE ===
