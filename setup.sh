#!/bin/bash
set -e

# ============================================================
#  Voice Translator — one-click setup
#  Usage:  git clone <repo> && cd LocalLLMVoiceProject && ./setup.sh
# ============================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()    { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC}  $*"; }
fail()  { echo -e "${RED}[FAIL]${NC}  $*"; exit 1; }

cd "$(dirname "$0")"
PROJECT_DIR="$(pwd)"
MODELS_DIR="$PROJECT_DIR/models"
mkdir -p "$MODELS_DIR"

echo ""
echo -e "${CYAN}══════════════════════════════════════════════${NC}"
echo -e "${CYAN}  Voice Translator — Setup${NC}"
echo -e "${CYAN}══════════════════════════════════════════════${NC}"
echo ""

# ─── 1. Install system dependencies ─────────────────────────
info "Checking and installing dependencies..."
echo ""

PACKAGES_TO_INSTALL=()

# Check what's missing
if ! command -v cmake &>/dev/null; then
    PACKAGES_TO_INSTALL+=(cmake)
fi

if ! command -v g++ &>/dev/null; then
    PACKAGES_TO_INSTALL+=(build-essential)
fi

if ! pkg-config --exists alsa 2>/dev/null; then
    PACKAGES_TO_INSTALL+=(libasound2-dev)
fi

if ! command -v curl &>/dev/null; then
    PACKAGES_TO_INSTALL+=(curl)
fi

if ! pkg-config --exists openssl 2>/dev/null; then
    PACKAGES_TO_INSTALL+=(libssl-dev pkg-config)
fi

# Install missing packages
if [ ${#PACKAGES_TO_INSTALL[@]} -gt 0 ]; then
    info "Installing: ${PACKAGES_TO_INSTALL[*]}"
    sudo apt-get update -qq
    sudo apt-get install -y -qq "${PACKAGES_TO_INSTALL[@]}"
    ok "System packages installed"
else
    ok "System packages — all present"
fi

# Rust
if command -v cargo &>/dev/null; then
    ok "Rust $(rustc --version | awk '{print $2}')"
else
    info "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
    ok "Rust $(rustc --version | awk '{print $2}') installed"
fi

# CUDA check
HAS_CUDA=false
if command -v nvcc &>/dev/null; then
    ok "CUDA $(nvcc --version | grep release | awk '{print $6}' | tr -d ',')"
    HAS_CUDA=true
else
    # Check if CUDA is installed but not in PATH
    if [ -d "/usr/local/cuda" ]; then
        export PATH="/usr/local/cuda/bin:$PATH"
        export LD_LIBRARY_PATH="/usr/local/cuda/lib64:$LD_LIBRARY_PATH"
        if command -v nvcc &>/dev/null; then
            ok "CUDA $(nvcc --version | grep release | awk '{print $6}' | tr -d ',') (added to PATH)"
            HAS_CUDA=true
        fi
    fi

    if [ "$HAS_CUDA" = false ]; then
        echo ""
        warn "CUDA not found!"
        warn "Translation LLM requires NVIDIA GPU + CUDA for reasonable speed."
        echo ""
        echo -e "  Install CUDA Toolkit:"
        echo -e "    ${CYAN}sudo apt install nvidia-cuda-toolkit${NC}"
        echo -e "  Or from NVIDIA:"
        echo -e "    ${CYAN}https://developer.nvidia.com/cuda-downloads${NC}"
        echo ""
        read -p "  Continue without CUDA? (translation will be VERY slow) [y/N] " -n 1 -r
        echo ""
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            fail "Install CUDA and re-run ./setup.sh"
        fi
    fi
fi

# Summary
echo ""
ok "Dependencies ready"
echo ""

# ─── 2. Download models ─────────────────────────────────────
info "Downloading models (total ~3.1 GB)..."
echo ""

download() {
    local url="$1"
    local dest="$2"
    local name="$3"

    if [ -f "$dest" ]; then
        ok "$name — already exists"
        return 0
    fi

    info "Downloading $name..."
    if curl -L --progress-bar -o "$dest.tmp" "$url"; then
        mv "$dest.tmp" "$dest"
        ok "$name — done ($(du -h "$dest" | cut -f1))"
    else
        rm -f "$dest.tmp"
        fail "Failed to download $name"
    fi
}

# Silero VAD (~630 KB)
download \
    "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx" \
    "$MODELS_DIR/silero_vad.onnx" \
    "Silero VAD v5 (~630 KB)"

# GigaAM-v3 STT (~320 MB)
download \
    "https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx/resolve/main/gigaam_v3_e2e_ctc_int8.onnx" \
    "$MODELS_DIR/gigaam_v3_e2e_ctc_int8.onnx" \
    "GigaAM-v3 STT INT8 (~320 MB)"

# Piper TTS (~63 MB)
download \
    "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx" \
    "$MODELS_DIR/piper-en_US-lessac-medium.onnx" \
    "Piper TTS en_US (~63 MB)"

# Qwen3.5-4B translation (~2.7 GB)
download \
    "https://huggingface.co/AaryanK/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf" \
    "$MODELS_DIR/Qwen3.5-4B-Q4_K_M.gguf" \
    "Qwen3.5-4B Q4_K_M (~2.7 GB)"

echo ""

# ─── 3. Build project ───────────────────────────────────────
info "Building project (release mode, this may take several minutes)..."
echo ""

cargo build --release -p voice-core 2>&1

echo ""
ok "Build complete!"

# ─── 4. Verify ───────────────────────────────────────────────
echo ""
info "Verifying installation..."

MISSING=()
[ ! -f "$MODELS_DIR/silero_vad.onnx" ] && MISSING+=("silero_vad.onnx")
[ ! -f "$MODELS_DIR/gigaam_v3_e2e_ctc_int8.onnx" ] && MISSING+=("gigaam_v3_e2e_ctc_int8.onnx")
[ ! -f "$MODELS_DIR/Qwen3.5-4B-Q4_K_M.gguf" ] && MISSING+=("Qwen3.5-4B-Q4_K_M.gguf")
[ ! -f "$PROJECT_DIR/target/release/voice-translator" ] && MISSING+=("voice-translator binary")

if [ ${#MISSING[@]} -gt 0 ]; then
    warn "Missing components: ${MISSING[*]}"
    fail "Setup incomplete. Check errors above."
fi

ok "All components verified"

# ─── 5. Done ─────────────────────────────────────────────────
echo ""
echo -e "${GREEN}══════════════════════════════════════════════${NC}"
echo -e "${GREEN}  Setup complete!${NC}"
echo -e "${GREEN}══════════════════════════════════════════════${NC}"
echo ""
echo -e "  Start the translator:"
echo -e "    ${CYAN}./run.sh${NC}"
echo ""
echo -e "  Other modes:"
echo -e "    ${CYAN}./run.sh --mode stt${NC}          # STT only (no translation)"
echo -e "    ${CYAN}./run.sh --mode passthrough${NC}  # Test audio devices"
echo -e "    ${CYAN}./run.sh --list-devices${NC}      # Show audio devices"
echo ""
