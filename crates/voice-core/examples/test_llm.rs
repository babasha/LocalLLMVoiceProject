use std::time::Instant;

use voice_core::config::AppConfig;
use voice_core::translation::engine::TranslationEngine;

/// Quick translation benchmark. Run with VOICE_PERF=1 to get per-call
/// `[PERF] TR:` lines (prompt/gen tok/s) from the engine itself.
///
/// The first call pays one-time CUDA graph warmup, so we do a warmup pass and
/// then average a few measured runs on a realistic sentence.
fn main() {
    let config = AppConfig::load_default().unwrap();
    println!("Model:        {}", config.translation.model_path);
    println!("prompt_style: '{}'", config.translation.prompt_style);
    println!("Loading model...");

    let engine = match TranslationEngine::new(&config.translation) {
        Ok(e) => e,
        Err(e) => {
            println!("Load error: {e}");
            return;
        }
    };
    println!("Model loaded.");
    let mut ctx = engine.make_context().expect("Failed to create context");

    // A realistic, long-enough sentence so token/s numbers are meaningful
    // (a 2-word phrase generates too few tokens to time reliably).
    let sentence = "Сегодня я хочу рассказать вам о том, как работает синхронный \
                    перевод речи и почему задержка является главной проблемой.";

    // Warmup (discard timing): triggers CUDA graph capture / kernel autotune.
    println!("\n--- warmup ---");
    let _ = engine.translate_with(&mut ctx, "Привет мир", "ru", "en");

    println!("\n--- measured runs ---");
    let runs = 3;
    let mut total = 0.0f64;
    let mut last = String::new();
    for i in 1..=runs {
        let t = Instant::now();
        match engine.translate_with(&mut ctx, sentence, "ru", "en") {
            Ok(tr) => {
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                total += ms;
                last = tr;
                println!("run {i}: {ms:.0} ms");
            }
            Err(e) => println!("run {i}: error {e}"),
        }
    }
    println!("\navg wall time: {:.0} ms over {runs} runs", total / runs as f64);
    println!("translation:  '{last}'");
}
