//! UI language choice.

/// Languages with a bundled catalog; `en` is the source language.
pub const SUPPORTED: [&str; 4] = ["en", "ru", "zh_CN", "es"];

/// Supported language for an OS locale like `ru-RU` or `zh-Hans-CN`; `en` otherwise.
pub fn pick_language(locale: &str) -> &'static str {
    let prefix = locale.split(['-', '_']).next().unwrap_or_default();
    match prefix.to_ascii_lowercase().as_str() {
        "ru" => "ru",
        "zh" => "zh_CN",
        "es" => "es",
        _ => "en",
    }
}

/// The language to use at start: the saved one if supported, else derived from the OS locale.
pub fn initial_language(saved: &str, os_locale: Option<&str>) -> &'static str {
    SUPPORTED
        .into_iter()
        .find(|&l| l == saved)
        .unwrap_or_else(|| pick_language(os_locale.unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_language_by_prefix() {
        assert_eq!(pick_language("ru-RU"), "ru");
        assert_eq!(pick_language("zh-Hans-CN"), "zh_CN");
        assert_eq!(pick_language("es-MX"), "es");
        assert_eq!(pick_language("de-DE"), "en");
    }

    #[test]
    fn pick_language_edge_cases() {
        assert_eq!(pick_language("ru"), "ru");
        assert_eq!(pick_language("zh_TW"), "zh_CN");
        assert_eq!(pick_language("ES-es"), "es");
        assert_eq!(pick_language("en-US"), "en");
        assert_eq!(pick_language(""), "en");
        assert_eq!(pick_language("rus"), "en");
    }

    #[test]
    fn initial_language_prefers_saved_supported_language() {
        assert_eq!(initial_language("es", Some("ru-RU")), "es");
        assert_eq!(initial_language("en", Some("ru-RU")), "en");
        assert_eq!(initial_language("", Some("zh-Hans-CN")), "zh_CN");
        assert_eq!(initial_language("", None), "en");
        // Unknown saved value falls back to the OS locale.
        assert_eq!(initial_language("fr", Some("ru-RU")), "ru");
    }
}
