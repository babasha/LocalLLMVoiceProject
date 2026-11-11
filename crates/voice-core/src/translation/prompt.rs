//! Prompt text for the translation LLM.

/// Build a translation prompt for the given language pair.
pub fn build_translation_prompt(text: &str, source_lang: &str, target_lang: &str) -> String {
    let source_name = lang_name(source_lang);
    let target_name = lang_name(target_lang);

    format!(
        "Translate the following {source_name} text to {target_name}. \
         Output ONLY the translation, nothing else.\n\n\
         {text}"
    )
}

/// Build a system prompt for the translator.
pub fn system_prompt(source_lang: &str, target_lang: &str) -> String {
    let source_name = lang_name(source_lang);
    let target_name = lang_name(target_lang);

    format!(
        "You are a simultaneous interpreter from {source_name} to {target_name}. \
         The input is live speech and may be an UNFINISHED utterance that is still \
         being spoken. Translate only what is actually given — never guess, \
         complete, or invent the rest of the sentence. If the assistant turn \
         already begins with a partial translation, continue it seamlessly \
         without repeating or rephrasing what is already there. \
         Translate accurately and naturally. Output ONLY the translation. \
         Do not add explanations, notes, or formatting."
    )
}

/// True when `text` is evidently already written in `target_lang`, judged by
/// script — so the LLM call can be skipped (e.g. the loopback channel picks up
/// Russian audio while translating *into* Russian). Only languages with a
/// distinctive script are detected; Latin-script pairs (en/de/fr...) are never
/// skipped because the script alone cannot tell them apart.
pub fn is_already_target(text: &str, target_lang: &str) -> bool {
    let in_script: fn(char) -> bool = match target_lang {
        "ru" | "uk" | "be" | "bg" | "sr" => |c| ('\u{0400}'..='\u{04FF}').contains(&c),
        "zh" => |c| ('\u{4E00}'..='\u{9FFF}').contains(&c),
        "ja" => |c| ('\u{3040}'..='\u{30FF}').contains(&c) || ('\u{4E00}'..='\u{9FFF}').contains(&c),
        "ko" => |c| ('\u{AC00}'..='\u{D7AF}').contains(&c),
        "ar" => |c| ('\u{0600}'..='\u{06FF}').contains(&c),
        "he" => |c| ('\u{0590}'..='\u{05FF}').contains(&c),
        "el" => |c| ('\u{0370}'..='\u{03FF}').contains(&c),
        _ => return false,
    };
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() < 3 {
        return false;
    }
    let hits = letters.iter().filter(|&&c| in_script(c)).count();
    hits * 10 >= letters.len() * 8
}

fn lang_name(code: &str) -> &str {
    match code {
        "ru" => "Russian",
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        _ => code,
    }
}

#[cfg(test)]
mod tests {
    use super::is_already_target;

    #[test]
    fn detects_script() {
        assert!(is_already_target("Ну тут и бариста нет.", "ru"));
        assert!(!is_already_target("Alles hat ein Ende.", "ru"));
        assert!(!is_already_target("Привет, Bob and Alice and Charlie", "ru"));
        assert!(!is_already_target("Hello there", "en")); // Latin: never skipped
    }
}
