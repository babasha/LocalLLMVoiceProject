use std::num::NonZeroU32;
use std::pin::pin;
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, info};

use crate::config::TranslationConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::translation::prompt;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

/// Chat-template family for prompt construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptStyle {
    /// Qwen / ChatML: `<|im_start|>` roles, optional `<think>` block.
    ChatMl,
    /// Gemma: `<start_of_turn>` turns, no system role, no thinking.
    Gemma,
}

/// LLM-based translation engine (Qwen3.5 / Gemma 4) via llama.cpp.
pub struct TranslationEngine {
    config: TranslationConfig,
    backend: Arc<LlamaBackend>,
    model: Arc<LlamaModel>,
}

impl TranslationEngine {
    /// Initialize the translation engine — loads the GGUF model into GPU.
    pub fn new(config: &TranslationConfig) -> Result<Self> {
        info!("Initializing LLM backend...");
        let mut backend = LlamaBackend::init().map_err(|e| {
            VoiceTranslatorError::Translation(format!("Failed to init llama backend: {e}"))
        })?;
        // Suppress llama.cpp/ggml internal log spam (CUDA graph warmup, etc.)
        // Keep logs when VOICE_LLAMA_LOGS is set (diagnostics).
        if std::env::var("VOICE_LLAMA_LOGS").is_err() {
            backend.void_logs();
        }

        let model_params = pin!(LlamaModelParams::default()
            .with_n_gpu_layers(config.n_gpu_layers));

        info!("Loading model: {}", config.model_path);
        let model =
            LlamaModel::load_from_file(&backend, &config.model_path, &model_params).map_err(
                |e| VoiceTranslatorError::Translation(format!("Failed to load model: {e}")),
            )?;

        info!(
            "Model loaded. Vocab size: {}, GPU layers: {}",
            model.n_vocab(),
            config.n_gpu_layers
        );

        Ok(TranslationEngine {
            config: config.clone(),
            backend: Arc::new(backend),
            model: Arc::new(model),
        })
    }

    /// Create a persistent context to reuse across multiple translate calls.
    /// Call this once, then pass the returned context to `translate_with`.
    pub fn make_context(&self) -> Result<LlamaContext<'_>> {
        let ctx_size = NonZeroU32::new(self.config.context_size)
            .unwrap_or(NonZeroU32::new(4096).unwrap());
        let ctx_params = LlamaContextParams::default().with_n_ctx(Some(ctx_size));

        self.model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| {
                VoiceTranslatorError::Translation(format!("Failed to create context: {e}"))
            })
    }

    /// Translate using a pre-created context (no re-allocation between calls).
    /// Safe to reuse: decodes from position 0 each time, overwriting KV cache entries.
    pub fn translate_with(
        &self,
        ctx: &mut LlamaContext<'_>,
        text: &str,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<String> {
        self.translate_prefixed(ctx, text, "", source_lang, target_lang)
    }

    /// Incremental ("simultaneous") translation: prefill the assistant turn with
    /// `prefix` — the English we have *already spoken aloud* — so the model is
    /// forced to *continue* from it instead of re-translating the growing
    /// utterance from scratch. Because the committed prefix is locked into the
    /// prompt, the model can never contradict words the listener already heard,
    /// and each newly generated word can be voiced almost immediately.
    ///
    /// Returns the full translation (`prefix` + freshly generated continuation),
    /// so callers can diff against `prefix` to find the new tail.
    pub fn translate_prefixed(
        &self,
        ctx: &mut LlamaContext<'_>,
        text: &str,
        prefix: &str,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        let prefix = prefix.trim();

        // Clear KV cache before each new prompt — required for M-RoPE (Qwen3.5):
        // positions must be monotonically increasing, so we reset to 0 each call.
        ctx.clear_kv_cache();

        let full_prompt = self.build_chat_prompt(trimmed, prefix, source_lang, target_lang);
        debug!("Prompt: {}", full_prompt);

        let tokens = self
            .model
            .str_to_token(&full_prompt, AddBos::Always)
            .map_err(|e| {
                VoiceTranslatorError::Translation(format!("Tokenization failed: {e}"))
            })?;

        debug!("Prompt tokens: {}", tokens.len());

        // Decode prompt starting at position 0 — overwrites previous KV cache entries
        let mut batch = LlamaBatch::new(self.config.context_size as usize, 1);
        let last_idx = (tokens.len() - 1) as i32;
        for (i, token) in (0_i32..).zip(tokens.iter()) {
            batch.add(*token, i, &[0], i == last_idx).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Batch add failed: {e}"))
            })?;
        }

        let perf = std::env::var("VOICE_PERF").is_ok();
        let t_prompt = perf.then(Instant::now);
        let prompt_tokens = tokens.len();

        ctx.decode(&mut batch).map_err(|e| {
            VoiceTranslatorError::Translation(format!("Prompt decode failed: {e}"))
        })?;
        let prompt_ms = t_prompt.map(|t| t.elapsed().as_secs_f64() * 1000.0);
        let t_gen = perf.then(Instant::now);
        let mut gen_tokens = 0usize;

        // Greedy (argmax) when temperature <= 0: deterministic decoding keeps
        // consecutive partial translations stable, which is essential for
        // streaming — a sampled re-translation would jitter the prefix and
        // strand words that should have been voiced. Fall back to the sampled
        // chain only if a positive temperature is explicitly configured.
        let mut sampler = if self.config.temperature <= 0.0 {
            LlamaSampler::greedy()
        } else {
            LlamaSampler::chain_simple([
                LlamaSampler::temp(self.config.temperature),
                LlamaSampler::top_k(20),
                LlamaSampler::top_p(0.8, 1),
                LlamaSampler::dist(42),
            ])
        };

        let mut output = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut n_cur = batch.n_tokens();
        let max_tokens = self.config.max_tokens as i32;

        for _ in 0..max_tokens {
            let token = sampler.sample(ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            if self.model.is_eog_token(token) {
                break;
            }

            let piece = self
                .model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|e| {
                    VoiceTranslatorError::Translation(format!("Token decode failed: {e}"))
                })?;

            if output.contains("<think>")
                || piece.contains("<|im_end|>")
                || piece.contains("<end_of_turn>")
            {
                break;
            }

            output.push_str(&piece);
            gen_tokens += 1;

            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Batch add failed: {e}"))
            })?;

            ctx.decode(&mut batch).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Decode failed: {e}"))
            })?;

            n_cur += 1;
        }

        if let (Some(pms), Some(tg)) = (prompt_ms, t_gen) {
            let gen_ms = tg.elapsed().as_secs_f64() * 1000.0;
            let tps = if gen_ms > 0.0 { gen_tokens as f64 / (gen_ms / 1000.0) } else { 0.0 };
            eprintln!(
                "[PERF] TR: prompt {prompt_tokens}tok/{pms:.0}ms, gen {gen_tokens}tok/{gen_ms:.0}ms = {tps:.1} tok/s"
            );
        }

        // The model only generated the continuation; re-attach the forced
        // prefix so callers receive the full translation and can diff for the
        // newly settled tail.
        let continuation = self.clean_output(&output);
        let result = if prefix.is_empty() {
            continuation
        } else if continuation.is_empty() {
            prefix.to_string()
        } else {
            format!("{prefix} {continuation}")
        };
        debug!("Translation result: '{}'", result);
        Ok(result)
    }

    /// Which chat template to emit. Resolved from `prompt_style` config, or
    /// auto-detected from the model filename when that is blank.
    fn prompt_style(&self) -> PromptStyle {
        let hint = if self.config.prompt_style.is_empty() {
            self.config.model_path.to_lowercase()
        } else {
            self.config.prompt_style.to_lowercase()
        };
        if hint.contains("gemma") {
            PromptStyle::Gemma
        } else {
            PromptStyle::ChatMl
        }
    }

    fn build_chat_prompt(
        &self,
        text: &str,
        prefix: &str,
        source_lang: &str,
        target_lang: &str,
    ) -> String {
        let system = prompt::system_prompt(source_lang, target_lang);
        let user = prompt::build_translation_prompt(text, source_lang, target_lang);

        // The assistant turn is prefilled with `prefix` — the English we've
        // already spoken — so the model *continues* it (incremental/simultaneous
        // decoding) instead of re-emitting the whole sentence. No trailing space:
        // the next generated token piece carries its own leading space.
        let head = match self.prompt_style() {
            // Gemma has no system role and no <think>: fold the instructions into
            // the single user turn. BOS is added by AddBos::Always, so we must
            // NOT emit a literal <bos> here. End-of-turn is an EOG token, so the
            // generation loop stops on it via is_eog_token.
            PromptStyle::Gemma => format!(
                "<start_of_turn>user\n{system}\n\n{user}<end_of_turn>\n\
                 <start_of_turn>model\n"
            ),
            PromptStyle::ChatMl if self.config.enable_thinking => format!(
                "<|im_start|>system\n{system}<|im_end|>\n\
                 <|im_start|>user\n{user}<|im_end|>\n\
                 <|im_start|>assistant\n"
            ),
            PromptStyle::ChatMl => format!(
                "<|im_start|>system\n{system}<|im_end|>\n\
                 <|im_start|>user\n{user}<|im_end|>\n\
                 <|im_start|>assistant\n<think>\n</think>\n"
            ),
        };
        format!("{head}{prefix}")
    }

    fn clean_output(&self, raw: &str) -> String {
        let mut s = raw.to_string();

        if let Some(start) = s.find("<think>") {
            if let Some(end) = s.find("</think>") {
                s = format!("{}{}", &s[..start], &s[end + 8..]);
            } else {
                s = s[..start].to_string();
            }
        }

        s = s
            .replace("<|im_end|>", "")
            .replace("<|im_start|>", "")
            .replace("<|endoftext|>", "")
            .replace("<end_of_turn>", "")
            .replace("<start_of_turn>", "");

        let trimmed = s.trim();
        let trimmed = trimmed.strip_prefix('"').unwrap_or(trimmed);
        let trimmed = trimmed.strip_suffix('"').unwrap_or(trimmed);
        trimmed.trim().to_string()
    }
}
