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
    windows_resources();
}

/// Executable icon (drawn by `tools/gen_icon.py`) and version information.
fn windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/app.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/app.ico")
        .set("FileDescription", "TimeTracker")
        .set("ProductName", "TimeTracker")
        .set("OriginalFilename", "tt-app.exe");
    res.compile().expect("compile Windows resources");
}
