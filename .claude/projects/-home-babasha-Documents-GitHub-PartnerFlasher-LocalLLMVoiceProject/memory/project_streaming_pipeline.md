---
name: streaming-pipeline-status
description: Real-time streaming STT + translation pipeline is working end-to-end
type: project
---

Streaming pipeline is operational as of 2026-03-21:
- GigaAM-v3 (sherpa-onnx) → streaming STT with partial results every ~1s
- Qwen3.5-4B (llama-cpp-2, Q4_K_M GGUF, full GPU offload) → parallel streaming translation
- Both [RU] and [EN] lines update in real-time on terminal
- Adaptive silence detection: 2s base, 4s for long utterances (>3s speech)
- Force-flush at 15s continuous speech
- `ctx.clear_kv_cache()` before each translation (M-RoPE requirement)
- `backend.void_logs()` suppresses CUDA graph warmup spam
- Partial deduplication: only sends to translation when STT text actually changed

**Why:** User wants real-time voice translation (Russian → English) running locally on GPU.
**How to apply:** Pipeline is in `crates/voice-core/src/pipeline/orchestrator.rs`, translation in `crates/voice-core/src/translation/engine.rs`. Config in `config/default.toml`.
