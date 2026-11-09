use std::path::PathBuf;

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    let translations = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("..")
        .join("translations");
    println!("cargo:rerun-if-changed={}", translations.display());
    // Hand-maintained catalogs: strings without an explicit `"ctx" =>` carry no
    // msgctxt (instead of the enclosing component name).
    let config = CompilerConfiguration::new()
        .with_bundled_translations(translations)
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/main.slint", config).unwrap();
}
