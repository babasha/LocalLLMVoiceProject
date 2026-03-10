#!/bin/bash
cd "$(dirname "$0")"
LD_LIBRARY_PATH=tools/llama-cpp-cuda tools/llama-cpp-cuda/llama-cli -m models/Qwen3.5-4B-Q4_K_M.gguf -ngl 99 -c 4096 -cnv --chat-template chatml -t 8 -sys "You are a helpful assistant."
