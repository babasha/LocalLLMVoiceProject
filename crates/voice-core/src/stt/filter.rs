//! Garbage / hallucination filter for STT output.
//!
//! Sources of junk we guard against:
//!   * near-silent segments (clicks, breathing, fan noise) that VAD let through —
//!     rejected before decoding by an RMS gate, which also saves GPU/CPU time;
//!   * output with no letters/digits at all ("...", "-", "?");
//!   * decoder loops ("да да да да да да", "thank you thank you thank you") —
//!     collapsed to a bounded number of repeats;
//!   * well-known Whisper training-data hallucinations ("Продолжение следует...",
//!     "Thanks for watching!", subtitle credits) — dropped when they are the
//!     whole result, stripped when they lead/trail real speech.

use crate::config::SttFilterConfig;

/// Phrases Whisper-family models emit on silence/music. Compared after
/// normalization (lowercase, letters/digits/spaces only).
const BUILTIN_BLOCKLIST: &[&str] = &[
    "продолжение следует",
    "субтитры сделал dimatorzok",
    "субтитры создавал dimatorzok",
    "субтитры делал dimatorzok",
    "редактор субтитров а синецкая корректор а егорова",
    "редактор субтитров а синецкая",
    "корректор а егорова",
    "спасибо за просмотр",
    "подписывайтесь на канал",
    "thanks for watching",
    "thank you for watching",
    "please subscribe",
    "subtitles by the amaraorg community",
    "transcription by castingwords",
    "ご視聴ありがとうございました",
    "字幕由amaraorg社区提供",
];

pub struct SttFilter {
    min_rms: f32,
    max_repeats: usize,
    blocklist: Vec<String>,
}

impl SttFilter {
    pub fn new(cfg: &SttFilterConfig) -> Self {
        let mut blocklist: Vec<String> = BUILTIN_BLOCKLIST.iter().map(|s| normalize(s)).collect();
        blocklist.extend(cfg.blocklist.iter().map(|s| normalize(s)).filter(|s| !s.is_empty()));
        // Longest first so a long credit line is stripped before a shorter
        // phrase that it contains.
        blocklist.sort_by_key(|s| std::cmp::Reverse(s.len()));
        SttFilter {
            min_rms: cfg.min_rms,
            max_repeats: cfg.max_repeats.max(1),
            blocklist,
        }
    }

    /// False if the segment is too quiet to contain speech.
    pub fn audio_has_speech(&self, audio: &[f32]) -> bool {
        if audio.is_empty() {
            return false;
        }
        rms(audio) >= self.min_rms
    }

    /// Clean a raw transcription. Returns "" if nothing useful is left.
    pub fn clean(&self, raw: &str) -> String {
        let text = raw.trim();
        if !text.chars().any(char::is_alphanumeric) {
            return String::new();
        }
        let text = collapse_repeats(text, self.max_repeats);
        let text = self.strip_blocklisted(&text);
        if !text.chars().any(char::is_alphanumeric) {
            return String::new();
        }
        text
    }

    fn strip_blocklisted(&self, text: &str) -> String {
        let mut words: Vec<&str> = text.split_whitespace().collect();
        loop {
            let norm = normalize(&words.join(" "));
            if norm.is_empty() {
                return String::new();
            }
            let mut changed = false;
            for phrase in &self.blocklist {
                if norm == *phrase {
                    return String::new();
                }
                let n = phrase.split(' ').count();
                if words.len() > n {
                    if normalize(&words[..n].join(" ")) == *phrase {
                        words.drain(..n);
                        changed = true;
                        break;
                    }
                    if normalize(&words[words.len() - n..].join(" ")) == *phrase {
                        words.truncate(words.len() - n);
                        changed = true;
                        break;
                    }
                }
            }
            if !changed {
                return words.join(" ");
            }
        }
    }
}

pub fn rms(audio: &[f32]) -> f32 {
    if audio.is_empty() {
        return 0.0;
    }
    (audio.iter().map(|s| s * s).sum::<f32>() / audio.len() as f32).sqrt()
}

/// Lowercase, keep only letters/digits, single-space separated.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Collapse an n-gram (n = 1..=4) repeated more than `max` times in a row down
/// to `max` occurrences. Comparison ignores case and punctuation.
fn collapse_repeats(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let keys: Vec<String> = words.iter().map(|w| normalize(w)).collect();
    let mut out: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let mut collapsed = false;
        for n in 1..=4usize {
            if i + n > words.len() {
                break;
            }
            let gram = &keys[i..i + n];
            if gram.iter().all(String::is_empty) {
                continue;
            }
            let mut reps = 1;
            while i + (reps + 1) * n <= words.len() && keys[i + reps * n..i + (reps + 1) * n] == *gram {
                reps += 1;
            }
            if reps > max {
                out.extend_from_slice(&words[i..i + n * max]);
                i += n * reps;
                collapsed = true;
                break;
            }
        }
        if !collapsed {
            out.push(words[i]);
            i += 1;
        }
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> SttFilter {
        SttFilter::new(&SttFilterConfig::default())
    }

    #[test]
    fn drops_punctuation_only() {
        assert_eq!(f().clean(" ... "), "");
        assert_eq!(f().clean("?"), "");
    }

    #[test]
    fn keeps_normal_speech() {
        assert_eq!(f().clean("Привет, как дела?"), "Привет, как дела?");
        assert_eq!(f().clean("да да"), "да да");
    }

    #[test]
    fn collapses_loops() {
        assert_eq!(f().clean("да да да да да да да"), "да да да");
        assert_eq!(
            f().clean("thank you thank you thank you thank you thank you"),
            "thank you thank you thank you"
        );
    }

    #[test]
    fn drops_whole_hallucination() {
        assert_eq!(f().clean("Продолжение следует..."), "");
        assert_eq!(f().clean("Thanks for watching!"), "");
        assert_eq!(f().clean("Субтитры сделал DimaTorzok"), "");
    }

    #[test]
    fn strips_leading_and_trailing_hallucination() {
        assert_eq!(f().clean("Хорошо, договорились. Продолжение следует..."), "Хорошо, договорились.");
        assert_eq!(f().clean("Thanks for watching! See you"), "See you");
    }

    #[test]
    fn custom_blocklist() {
        let cfg = SttFilterConfig { blocklist: vec!["Угу".into()], ..Default::default() };
        assert_eq!(SttFilter::new(&cfg).clean("угу."), "");
    }

    #[test]
    fn rms_gate() {
        let quiet = vec![0.001f32; 16000];
        let loud: Vec<f32> = (0..16000).map(|i| ((i as f32) * 0.05).sin() * 0.2).collect();
        assert!(!f().audio_has_speech(&quiet));
        assert!(f().audio_has_speech(&loud));
    }
}
