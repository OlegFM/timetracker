//! User settings persisted as individual string keys in the `settings` table.

use chrono::Weekday;
use rusqlite::Connection;
use tt_domain::{FormatPrefs, ParseKeyError, Tz};
use tt_storage::{Db, settings as kv};

use crate::error::Result;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

impl std::str::FromStr for Theme {
    type Err = ParseKeyError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "dark" => Ok(Theme::Dark),
            "light" => Ok(Theme::Light),
            _ => Err(ParseKeyError),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// UI language code; empty = not chosen yet.
    pub language: String,
    /// `hour_suffix` / `minute_suffix` are not persisted (filled by the UI).
    pub format: FormatPrefs,
    pub theme: Theme,
    pub mini_timer_visible: bool,
    pub mini_timer_idle_opacity: f32,
    pub mini_timer_pos: Option<(i32, i32)>,
    pub close_to_tray: bool,
    pub autostart: bool,
    pub prevent_overlap: bool,
    /// Cents per hour.
    pub default_rate: Option<i64>,
    pub update_auto_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: String::new(),
            format: FormatPrefs {
                tz: system_tz(),
                ..FormatPrefs::default()
            },
            theme: Theme::Dark,
            mini_timer_visible: true,
            mini_timer_idle_opacity: 0.3,
            mini_timer_pos: None,
            close_to_tray: true,
            autostart: false,
            prevent_overlap: false,
            default_rate: None,
            update_auto_check: true,
        }
    }
}

/// The operating system's time zone, UTC if unknown.
pub fn system_tz() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(Tz::UTC)
}

mod keys {
    pub const LANGUAGE: &str = "language";
    pub const DATE: &str = "format.date";
    pub const TIME: &str = "format.time";
    pub const INTERVAL: &str = "format.interval";
    pub const NUMBER: &str = "format.number";
    pub const CURRENCY: &str = "format.currency";
    pub const CURRENCY_CODE: &str = "format.currency-code";
    pub const WEEK_START: &str = "format.week-start";
    pub const TZ: &str = "format.tz";
    pub const THEME: &str = "theme";
    pub const MINI_VISIBLE: &str = "mini-timer.visible";
    pub const MINI_OPACITY: &str = "mini-timer.idle-opacity";
    pub const MINI_POS: &str = "mini-timer.pos";
    pub const CLOSE_TO_TRAY: &str = "close-to-tray";
    pub const AUTOSTART: &str = "autostart";
    pub const PREVENT_OVERLAP: &str = "prevent-overlap";
    pub const DEFAULT_RATE: &str = "default-rate";
    pub const UPDATE_AUTO_CHECK: &str = "update.auto-check";
}

const WEEKDAYS: [(Weekday, &str); 7] = [
    (Weekday::Mon, "mon"),
    (Weekday::Tue, "tue"),
    (Weekday::Wed, "wed"),
    (Weekday::Thu, "thu"),
    (Weekday::Fri, "fri"),
    (Weekday::Sat, "sat"),
    (Weekday::Sun, "sun"),
];

fn weekday_key(w: Weekday) -> &'static str {
    WEEKDAYS[w.num_days_from_monday() as usize].1
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    WEEKDAYS.iter().find(|(_, k)| *k == s).map(|(d, _)| *d)
}

fn parse_pos(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn parse_opacity(s: &str) -> Option<f32> {
    s.parse().ok().filter(|v| (0.0..=1.0).contains(v))
}

/// The stored value of `key` if present and accepted by `parse`.
fn read<T>(c: &Connection, key: &str, parse: impl FnOnce(&str) -> Option<T>) -> Result<Option<T>> {
    Ok(kv::get(c, key)?.as_deref().and_then(parse))
}

fn parsed<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.parse().ok()
}

pub struct SettingsService;

impl SettingsService {
    /// Missing or unparsable values fall back to defaults.
    pub fn load(db: &Db) -> Result<Settings> {
        let c = db.conn();
        let d = Settings::default();
        Ok(Settings {
            language: read(c, keys::LANGUAGE, |s| Some(s.to_owned()))?.unwrap_or(d.language),
            format: FormatPrefs {
                date: read(c, keys::DATE, parsed)?.unwrap_or(d.format.date),
                time: read(c, keys::TIME, parsed)?.unwrap_or(d.format.time),
                interval: read(c, keys::INTERVAL, parsed)?.unwrap_or(d.format.interval),
                number: read(c, keys::NUMBER, parsed)?.unwrap_or(d.format.number),
                currency: read(c, keys::CURRENCY, parsed)?.unwrap_or(d.format.currency),
                currency_code: read(c, keys::CURRENCY_CODE, |s| {
                    Some(s.to_owned()).filter(|s| !s.is_empty())
                })?
                .unwrap_or(d.format.currency_code),
                week_start: read(c, keys::WEEK_START, parse_weekday)?
                    .unwrap_or(d.format.week_start),
                tz: read(c, keys::TZ, parsed)?.unwrap_or(d.format.tz),
                ..d.format
            },
            theme: read(c, keys::THEME, parsed)?.unwrap_or(d.theme),
            mini_timer_visible: read(c, keys::MINI_VISIBLE, parsed)?
                .unwrap_or(d.mini_timer_visible),
            mini_timer_idle_opacity: read(c, keys::MINI_OPACITY, parse_opacity)?
                .unwrap_or(d.mini_timer_idle_opacity),
            mini_timer_pos: read(c, keys::MINI_POS, parse_pos)?.or(d.mini_timer_pos),
            close_to_tray: read(c, keys::CLOSE_TO_TRAY, parsed)?.unwrap_or(d.close_to_tray),
            autostart: read(c, keys::AUTOSTART, parsed)?.unwrap_or(d.autostart),
            prevent_overlap: read(c, keys::PREVENT_OVERLAP, parsed)?.unwrap_or(d.prevent_overlap),
            default_rate: read(c, keys::DEFAULT_RATE, parsed)?.or(d.default_rate),
            update_auto_check: read(c, keys::UPDATE_AUTO_CHECK, parsed)?
                .unwrap_or(d.update_auto_check),
        })
    }

    /// Writes every persisted key in one transaction. `None` values are stored as "".
    pub fn save(db: &mut Db, s: &Settings) -> Result<()> {
        let f = &s.format;
        let pairs = [
            (keys::LANGUAGE, s.language.clone()),
            (keys::DATE, f.date.as_str().to_owned()),
            (keys::TIME, f.time.as_str().to_owned()),
            (keys::INTERVAL, f.interval.as_str().to_owned()),
            (keys::NUMBER, f.number.as_str().to_owned()),
            (keys::CURRENCY, f.currency.as_str().to_owned()),
            (keys::CURRENCY_CODE, f.currency_code.clone()),
            (keys::WEEK_START, weekday_key(f.week_start).to_owned()),
            (keys::TZ, f.tz.name().to_owned()),
            (keys::THEME, s.theme.as_str().to_owned()),
            (keys::MINI_VISIBLE, s.mini_timer_visible.to_string()),
            (keys::MINI_OPACITY, s.mini_timer_idle_opacity.to_string()),
            (
                keys::MINI_POS,
                s.mini_timer_pos
                    .map_or(String::new(), |(x, y)| format!("{x},{y}")),
            ),
            (keys::CLOSE_TO_TRAY, s.close_to_tray.to_string()),
            (keys::AUTOSTART, s.autostart.to_string()),
            (keys::PREVENT_OVERLAP, s.prevent_overlap.to_string()),
            (
                keys::DEFAULT_RATE,
                s.default_rate.map_or(String::new(), |r| r.to_string()),
            ),
            (keys::UPDATE_AUTO_CHECK, s.update_auto_check.to_string()),
        ];
        db.transaction(|tx| {
            for (key, value) in &pairs {
                kv::set(tx, key, value)?;
            }
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_domain::{CurrencyFormat, DateFormat, IntervalFormat, NumberFormat, TimeFormat};

    #[test]
    fn defaults_on_empty_db() {
        let db = Db::open_in_memory().unwrap();
        let s = SettingsService::load(&db).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.language, "");
        assert_eq!(s.theme, Theme::Dark);
        assert_eq!(s.mini_timer_idle_opacity, 0.3);
        assert!(s.mini_timer_visible && s.close_to_tray && s.update_auto_check);
        assert!(!s.autostart && !s.prevent_overlap);
        assert_eq!((s.mini_timer_pos, s.default_rate), (None, None));
        assert_eq!(s.format.tz, system_tz());
    }

    #[test]
    fn save_load_round_trip() {
        let mut db = Db::open_in_memory().unwrap();
        let s = Settings {
            language: "ru".into(),
            format: FormatPrefs {
                date: DateFormat::PointDMY,
                time: TimeFormat::H12,
                interval: IntervalFormat::Decimal,
                number: NumberFormat::SpaceComma,
                currency: CurrencyFormat::IsoAfterSpace,
                currency_code: "EUR".into(),
                week_start: Weekday::Sun,
                tz: Tz::Asia__Tokyo,
                ..FormatPrefs::default()
            },
            theme: Theme::Light,
            mini_timer_visible: false,
            mini_timer_idle_opacity: 0.55,
            mini_timer_pos: Some((-120, 40)),
            close_to_tray: false,
            autostart: true,
            prevent_overlap: true,
            default_rate: Some(7_500),
            update_auto_check: false,
        };
        SettingsService::save(&mut db, &s).unwrap();
        assert_eq!(SettingsService::load(&db).unwrap(), s);
        assert_eq!(
            kv::get(db.conn(), "format.week-start").unwrap().as_deref(),
            Some("sun")
        );
        assert_eq!(
            kv::get(db.conn(), "mini-timer.pos").unwrap().as_deref(),
            Some("-120,40")
        );

        // Clearing optional values round-trips too.
        let cleared = Settings {
            mini_timer_pos: None,
            default_rate: None,
            ..s
        };
        SettingsService::save(&mut db, &cleared).unwrap();
        assert_eq!(SettingsService::load(&db).unwrap(), cleared);
    }

    #[test]
    fn garbage_value_falls_back_to_default() {
        let db = Db::open_in_memory().unwrap();
        for (key, value) in [
            ("format.date", "nope"),
            ("format.time", "25-hours"),
            ("format.interval", ""),
            ("format.number", "x"),
            ("format.currency", "x"),
            ("format.currency-code", ""),
            ("format.week-start", "funday"),
            ("format.tz", "Mars/Olympus"),
            ("theme", "purple"),
            ("mini-timer.visible", "yes"),
            ("mini-timer.idle-opacity", "abc"),
            ("mini-timer.pos", "1;2"),
            ("close-to-tray", "1"),
            ("autostart", "maybe"),
            ("prevent-overlap", "?"),
            ("default-rate", "ten"),
            ("update.auto-check", "no"),
        ] {
            kv::set(db.conn(), key, value).unwrap();
        }
        assert_eq!(SettingsService::load(&db).unwrap(), Settings::default());

        kv::set(db.conn(), "mini-timer.idle-opacity", "7").unwrap();
        assert_eq!(
            SettingsService::load(&db).unwrap().mini_timer_idle_opacity,
            0.3
        );
    }
}
