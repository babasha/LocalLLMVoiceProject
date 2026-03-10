use std::num::NonZeroU32;
use std::pin::pin;
use std::sync::Arc;

use tracing::{debug, info};

use crate::config::TranslationConfig;
use crate::error::{Result, VoiceTranslatorError};
use crate::translation::prompt;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

/// LLM-based translation engine using Qwen3.5-4B via llama.cpp.
/// Uses CUDA GPU acceleration for fast inference.
pub struct TranslationEngine {
    config: TranslationConfig,
    backend: Arc<LlamaBackend>,
    model: Arc<LlamaModel>,
}

impl TranslationEngine {
    /// Initialize the translation engine — loads the GGUF model into GPU.
    pub fn new(config: &TranslationConfig) -> Result<Self> {
        info!("Initializing LLM backend...");
        let backend = LlamaBackend::init().map_err(|e| {
            VoiceTranslatorError::Translation(format!("Failed to init llama backend: {e}"))
        })?;

        let model_params = pin!(LlamaModelParams::default()
            .with_n_gpu_layers(config.n_gpu_layers));

        info!("Loading model: {}", config.model_path);
        let model =
            LlamaModel::load_from_file(&backend, &config.model_path, &model_params).map_err(
                |e| {
                    VoiceTranslatorError::Translation(format!(
                        "Failed to load model '{}': {e}",
                        config.model_path
                    ))
                },
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

    /// Translate text from source to target language.
    pub fn translate(
        &self,
        text: &str,
        source_lang: &str,
        target_lang: &str,
    ) -> Result<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }

        // Build chat-style prompt for Qwen3.5
        let full_prompt = self.build_chat_prompt(trimmed, source_lang, target_lang);
        debug!("Prompt: {}", full_prompt);

        // Create context for this inference
        let ctx_size = NonZeroU32::new(self.config.context_size)
            .unwrap_or(NonZeroU32::new(4096).unwrap());
        let ctx_params = LlamaContextParams::default().with_n_ctx(Some(ctx_size));

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| {
                VoiceTranslatorError::Translation(format!("Failed to create context: {e}"))
            })?;

        // Tokenize prompt
        let tokens = self.model.str_to_token(&full_prompt, AddBos::Always).map_err(|e| {
            VoiceTranslatorError::Translation(format!("Tokenization failed: {e}"))
        })?;

        debug!("Prompt tokens: {}", tokens.len());

        // Feed prompt tokens into context
        let mut batch = LlamaBatch::new(self.config.context_size as usize, 1);
        let last_idx = (tokens.len() - 1) as i32;
        for (i, token) in (0_i32..).zip(tokens.iter()) {
            batch.add(*token, i, &[0], i == last_idx).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Batch add failed: {e}"))
            })?;
        }

        ctx.decode(&mut batch).map_err(|e| {
            VoiceTranslatorError::Translation(format!("Prompt decode failed: {e}"))
        })?;

        // Set up sampler
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(self.config.temperature),
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.8, 1),
            LlamaSampler::dist(42),
        ]);

        // Generate tokens
        let mut output = String::new();
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        let mut n_cur = batch.n_tokens();
        let max_tokens = self.config.max_tokens as i32;

        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            // Check for end of generation
            if self.model.is_eog_token(token) {
                break;
            }

            // Decode token to text
            let piece = self.model.token_to_piece(token, &mut decoder, true, None).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Token decode failed: {e}"))
            })?;

            // Stop if we hit thinking tags (shouldn't happen with thinking disabled)
            if output.contains("<think>") || piece.contains("<|im_end|>") {
                break;
            }

            output.push_str(&piece);

            // Prepare next token
            batch.clear();
            batch.add(token, n_cur, &[0], true).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Batch add failed: {e}"))
            })?;

            ctx.decode(&mut batch).map_err(|e| {
                VoiceTranslatorError::Translation(format!("Decode failed: {e}"))
            })?;

            n_cur += 1;
        }

        let result = self.clean_output(&output);
        debug!("Translation result: '{}'", result);
        Ok(result)
    }

    /// Build a ChatML-formatted prompt for Qwen3.5.
    fn build_chat_prompt(&self, text: &str, source_lang: &str, target_lang: &str) -> String {
        let system = prompt::system_prompt(source_lang, target_lang);
        let user = prompt::build_translation_prompt(text, source_lang, target_lang);

        if self.config.enable_thinking {
            format!(
                "<|im_start|>system\n{system}<|im_end|>\n\
                 <|im_start|>user\n{user}<|im_end|>\n\
                 <|im_start|>assistant\n"
            )
        } else {
            // Disable thinking by adding /no_think prefix
            format!(
                "<|im_start|>system\n{system}<|im_end|>\n\
                 <|im_start|>user\n{user}<|im_end|>\n\
                 <|im_start|>assistant\n<think>\n</think>\n"
            )
        }
    }

    /// Clean up model output — remove tags, extra whitespace, quotes.
    fn clean_output(&self, raw: &str) -> String {
        let mut s = raw.to_string();

        // Remove any thinking block that leaked through
        if let Some(start) = s.find("<think>") {
            if let Some(end) = s.find("</think>") {
                s = format!("{}{}", &s[..start], &s[end + 8..]);
            } else {
                s = s[..start].to_string();
            }
        }

        // Remove special tokens
        s = s.replace("<|im_end|>", "")
            .replace("<|im_start|>", "")
            .replace("<|endoftext|>", "");

        // Trim surrounding quotes and whitespace
        let trimmed = s.trim();
        let trimmed = trimmed.strip_prefix('"').unwrap_or(trimmed);
        let trimmed = trimmed.strip_suffix('"').unwrap_or(trimmed);
        trimmed.trim().to_string()
    }
}
