/// Generates translation prompts for Qwen3.5-2B.

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
        "You are a real-time translator from {source_name} to {target_name}. \
         Translate the user's speech accurately and naturally. \
         Output ONLY the translation. Do not add explanations, notes, or formatting."
    )
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
