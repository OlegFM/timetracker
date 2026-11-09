//! Format preferences for the UI.

use tt_domain::FormatPrefs;
use tt_services::Settings;

/// Format preferences from `settings` with the localized duration suffixes (read from the
/// `Tr` global by the caller).
pub fn prefs_from(settings: &Settings, hour_suffix: &str, minute_suffix: &str) -> FormatPrefs {
    FormatPrefs {
        hour_suffix: hour_suffix.to_owned(),
        minute_suffix: minute_suffix.to_owned(),
        ..settings.format.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_domain::{IntervalFormat, format_duration};

    #[test]
    fn prefs_from_fills_suffixes_and_keeps_formats() {
        let mut settings = Settings::default();
        settings.format.interval = IntervalFormat::HoursMinutes;
        settings.format.currency_code = "EUR".into();
        let p = prefs_from(&settings, "ч", "мин");
        assert_eq!(
            (p.hour_suffix.as_str(), p.minute_suffix.as_str()),
            ("ч", "мин")
        );
        assert_eq!(p.interval, IntervalFormat::HoursMinutes);
        assert_eq!(p.currency_code, "EUR");
        assert_eq!(p.tz, settings.format.tz);
        assert_eq!(format_duration(5_400, &p), "1ч 30мин");
    }
}
