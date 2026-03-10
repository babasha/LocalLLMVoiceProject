/// Sentence boundary detector for streaming LLM→TTS overlap.
/// Accumulates translated tokens and flushes when a sentence boundary is detected.
pub struct SentenceDetector {
    buffer: String,
    /// Minimum number of words before flushing on punctuation
    min_words: usize,
}

impl SentenceDetector {
    pub fn new(min_words: usize) -> Self {
        SentenceDetector {
            buffer: String::new(),
            min_words: min_words.max(1),
        }
    }

    /// Feed a token (or partial text) into the detector.
    /// Returns a sentence fragment if a boundary is detected.
    pub fn feed(&mut self, token: &str) -> Option<String> {
        self.buffer.push_str(token);

        // Check for sentence-ending punctuation
        let trimmed = self.buffer.trim();
        if trimmed.is_empty() {
            return None;
        }

        let word_count = trimmed.split_whitespace().count();
        let last_char = trimmed.chars().last()?;

        // Flush on strong boundaries (. ! ?)
        if matches!(last_char, '.' | '!' | '?') && word_count >= self.min_words {
            return Some(self.flush());
        }

        // Flush on weak boundaries (, ; :) only if we have enough words
        if matches!(last_char, ',' | ';' | ':') && word_count >= self.min_words * 2 {
            return Some(self.flush());
        }

        None
    }

    /// Force flush remaining buffer content.
    pub fn flush(&mut self) -> String {
        let result = self.buffer.trim().to_string();
        self.buffer.clear();
        result
    }

    /// Check if buffer has content.
    pub fn has_content(&self) -> bool {
        !self.buffer.trim().is_empty()
    }
}
