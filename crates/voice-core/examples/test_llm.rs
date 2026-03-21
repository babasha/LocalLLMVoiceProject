use voice_core::config::AppConfig;
use voice_core::translation::engine::TranslationEngine;

fn main() {
    let config = AppConfig::load_default().unwrap();
    println!("Model path: {}", config.translation.model_path);
    println!("Attempting to load model...");
    match TranslationEngine::new(&config.translation) {
        Ok(engine) => {
            println!("Model loaded!");
            let mut ctx = engine.make_context().expect("Failed to create context");
            match engine.translate_with(&mut ctx, "Привет мир", "ru", "en") {
                Ok(result) => println!("Translation: '{}'", result),
                Err(e) => println!("Translation error: {}", e),
            }
        }
        Err(e) => println!("Load error: {}", e),
    }
}
