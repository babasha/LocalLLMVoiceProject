@echo off
setlocal
REM === Native Windows build of voice-translator (MSVC + CUDA) ===
REM Needs: Rust (MSVC), VS 2022 Build Tools (C++ workload), CMake, CUDA Toolkit, LLVM.

REM --- MSVC environment (located via vswhere) ---
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" echo ERROR: Visual Studio Build Tools not found & exit /b 1
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%i"
if not defined VSINSTALL echo ERROR: MSVC C++ toolset not found & exit /b 1
call "%VSINSTALL%\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
where cl >nul 2>&1
if errorlevel 1 echo ERROR: MSVC environment could not be initialised & exit /b 1

REM --- CUDA / LLVM (the CUDA installer normally sets CUDA_PATH itself) ---
if not defined CUDA_PATH set "CUDA_PATH=C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3"
REM Prefer the regular LLVM install for bindgen (an inherited LIBCLANG_PATH may point at
REM another toolchain, e.g. an embedded SDK).
if exist "C:\Program Files\LLVM\bin\libclang.dll" set "LIBCLANG_PATH=C:\Program Files\LLVM\bin"
set "PATH=%CUDA_PATH%\bin;%USERPROFILE%\.cargo\bin;C:\Program Files\CMake\bin;%PATH%"

REM Ninja drives nvcc directly; the Visual Studio generator trips over CUDA targets.
if not defined CMAKE_GENERATOR set "CMAKE_GENERATOR=Ninja"
REM Optional: build CUDA kernels for one GPU architecture only (faster build),
REM e.g.  set CMAKE_CUDA_ARCHITECTURES=120  for RTX 50xx (Blackwell).

REM Prebuilt sherpa-onnx + onnxruntime shipped with the repo.
set "SHERPA_ONNX_LIB_DIR=%~dp0libs\sherpa-onnx-win"

REM Keep build artifacts out of a OneDrive-synced tree (sync locks break the build).
REM Override by setting CARGO_TARGET_DIR before calling this script.
set "ROOT=%~dp0"
if not defined CARGO_TARGET_DIR if /i not "%ROOT:OneDrive=%"=="%ROOT%" set "CARGO_TARGET_DIR=C:\cargo-target\voice-translator"

cd /d "%~dp0"
echo CUDA_PATH=%CUDA_PATH%
echo CARGO_TARGET_DIR=%CARGO_TARGET_DIR%
echo === BUILD START ===
cargo build --release -p voice-core %*
echo === BUILD EXIT %ERRORLEVEL% ===
exit /b %ERRORLEVEL%
