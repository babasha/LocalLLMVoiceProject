#!/bin/bash
cd "$(dirname "$0")"

PORT="${1:-8080}"

echo "Starting Qwen3.5-4B server on http://localhost:$PORT"
echo "OpenAI-compatible API: http://localhost:$PORT/v1/chat/completions"
echo ""
echo "Press Ctrl+C to stop"
echo ""

LD_LIBRARY_PATH=tools/llama-cpp-cuda \
  tools/llama-cpp-cuda/llama-server \
  -m models/Qwen3.5-4B-Q4_K_M.gguf \
  -ngl 99 \
  -c 32000 \
  --port "$PORT" \
  --chat-template chatml
