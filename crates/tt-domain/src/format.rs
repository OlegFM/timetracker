use std::fmt::Write as _;

use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc, Weekday};
use chrono_tz::Tz;

use crate::ids::Millis;

/// Error for an unknown enum key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseKeyError;

impl std::fmt::Display for ParseKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unknown key")
    }
}

impl std::error::Error for ParseKeyError {}

/// Declares a fieldless enum with string keys (`as_str` + `FromStr`).
macro_rules! key_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $key:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $key),+ }
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::format::ParseKeyError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($key => Ok(Self::$variant),)+
                    _ => Err($crate::format::ParseKeyError),
                }
            }
        }
    };
}
pub(crate) use key_enum;

key_enum! {
    DateFormat {
        PointDMY => "point-separated-d-m-yyyy",
        SlashMMDDYYYY => "slash-separated-mm-dd-yyyy",
        SlashDDMMYYYY => "slash-separated-dd-mm-yyyy",
        HyphenDDMMYYYY => "hyphen-separated-dd-mm-yyyy",
        HyphenMMDDYYYY => "hyphen-separated-mm-dd-yyyy",
        HyphenYYYYMMDD => "hyphen-separated-yyyy-mm-dd",
    }
}

key_enum! {
    TimeFormat {
        H12 => "12-hours",
        H24 => "24-hours",
    }
}

key_enum! {
    IntervalFormat {
        Decimal => "decimal",
        HoursMinutes => "hours-minutes",
        HoursMinutesColon => "hours-minutes-colon-separated",
        HoursMinutesSecondsColon => "hours-minutes-seconds-colon-separated",
    }
}

key_enum! {
    NumberFormat {
        PointComma => "point-comma",
        CommaPoint => "comma-point",
        SpaceComma => "space-comma",
        SpacePoint => "space-point",
        ApostrophePoint => "apostrophe-point",
    }
}

key_enum! {
    CurrencyFormat {
        IsoBeforeSpace => "iso-code-before-with-space",
        IsoAfterSpace => "iso-code-after-with-space",
        SymbolBefore => "symbol-before",
        SymbolAfter => "symbol-after",
        SymbolBeforeSpace => "symbol-before-with-space",
        SymbolAfterSpace => "symbol-after-with-space",
    }
}

/// User display preferences. Suffixes are localizable (filled by the UI layer).
#[derive(Debug, Clone, PartialEq)]
pub struct FormatPrefs {
    pub date: DateFormat,
    pub time: TimeFormat,
    pub interval: IntervalFormat,
    pub number: NumberFormat,
    pub currency: CurrencyFormat,
    pub currency_code: String,
    pub week_start: Weekday,
    pub tz: Tz,
    pub hour_suffix: String,
    pub minute_suffix: String,
}

impl Default for FormatPrefs {
    fn default() -> Self {
        Self {
            date: DateFormat::HyphenYYYYMMDD,
            time: TimeFormat::H24,
            interval: IntervalFormat::HoursMinutesColon,
            number: NumberFormat::CommaPoint,
            currency: CurrencyFormat::SymbolBefore,
            currency_code: "USD".into(),
            week_start: Weekday::Mon,
            tz: Tz::UTC,
            hour_suffix: "h".into(),
            minute_suffix: "min".into(),
        }
    }
}

pub fn format_date(d: NaiveDate, f: DateFormat) -> String {
    let (y, m, day) = (d.year(), d.month(), d.day());
    match f {
        DateFormat::PointDMY => format!("{day}.{m}.{y}"),
        DateFormat::SlashMMDDYYYY => format!("{m:02}/{day:02}/{y}"),
        DateFormat::SlashDDMMYYYY => format!("{day:02}/{m:02}/{y}"),
        DateFormat::HyphenDDMMYYYY => format!("{day:02}-{m:02}-{y}"),
        DateFormat::HyphenMMDDYYYY => format!("{m:02}-{day:02}-{y}"),
        DateFormat::HyphenYYYYMMDD => format!("{y}-{m:02}-{day:02}"),
    }
}

/// Local time of day in `prefs.tz`: "21:05" or "9:05 PM" (AM/PM not localized).
pub fn format_time(t: Millis, prefs: &FormatPrefs) -> String {
    let dt = DateTime::<Utc>::from_timestamp_millis(t)
        .unwrap_or_default()
        .with_timezone(&prefs.tz);
    let (h, m) = (dt.hour(), dt.minute());
    match prefs.time {
        TimeFormat::H24 => format!("{h:02}:{m:02}"),
        TimeFormat::H12 => {
            let suffix = if h < 12 { "AM" } else { "PM" };
            let h12 = if h % 12 == 0 { 12 } else { h % 12 };
            format!("{h12}:{m:02} {suffix}")
        }
    }
}

/// Timer clock "H:MM:SS".
pub fn format_clock(seconds: i64) -> String {
    let s = seconds.max(0);
    format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// Duration according to `prefs.interval`. Negative values are clamped to 0.
pub fn format_duration(seconds: i64, prefs: &FormatPrefs) -> String {
    let s = seconds.max(0);
    let (h, m) = (s / 3600, s % 3600 / 60);
    match prefs.interval {
        IntervalFormat::Decimal => format!(
            "{} {}",
            format_number(s as f64 / 3600.0, 2, prefs.number),
            prefs.hour_suffix
        ),
        IntervalFormat::HoursMinutes => {
            format!("{h}{} {m}{}", prefs.hour_suffix, prefs.minute_suffix)
        }
        IntervalFormat::HoursMinutesColon => format!("{h}:{m:02}"),
        IntervalFormat::HoursMinutesSecondsColon => format_clock(s),
    }
}

/// Rounds half away from zero to `decimals` and applies the number format.
/// Values that round to zero carry no minus sign.
pub fn format_number(n: f64, decimals: u8, f: NumberFormat) -> String {
    let (group, decimal) = match f {
        NumberFormat::PointComma => ('.', ','),
        NumberFormat::CommaPoint => (',', '.'),
        NumberFormat::SpaceComma => (' ', ','),
        NumberFormat::SpacePoint => (' ', '.'),
        NumberFormat::ApostrophePoint => ('\'', '.'),
    };
    let pow = 10u128.pow(u32::from(decimals));
    let scaled = (n.abs() * pow as f64).round() as u128;
    let (int_part, frac_part) = (scaled / pow, scaled % pow);

    let digits = int_part.to_string();
    let mut out = String::new();
    if n < 0.0 && scaled != 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(group);
        }
        out.push(c);
    }
    if decimals > 0 {
        out.push(decimal);
        let _ = write!(out, "{frac_part:0width$}", width = usize::from(decimals));
    }
    out
}

pub fn currency_symbol(code: &str) -> &str {
    match code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "RUB" => "₽",
        "CNY" | "JPY" => "¥",
        other => other,
    }
}

/// Amount in cents with the currency placed per `prefs.currency`.
pub fn format_money(cents: i64, prefs: &FormatPrefs) -> String {
    let amount = format_number(cents.unsigned_abs() as f64 / 100.0, 2, prefs.number);
    let code = prefs.currency_code.as_str();
    let symbol = currency_symbol(code);
    let body = match prefs.currency {
        CurrencyFormat::IsoBeforeSpace => format!("{code} {amount}"),
        CurrencyFormat::IsoAfterSpace => format!("{amount} {code}"),
        CurrencyFormat::SymbolBefore => format!("{symbol}{amount}"),
        CurrencyFormat::SymbolAfter => format!("{amount}{symbol}"),
        CurrencyFormat::SymbolBeforeSpace => format!("{symbol} {amount}"),
        CurrencyFormat::SymbolAfterSpace => format!("{amount} {symbol}"),
    };
    if cents < 0 { format!("-{body}") } else { body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use chrono_tz::Europe::Moscow;

    fn prefs() -> FormatPrefs {
        FormatPrefs::default()
    }

    fn utc(h: u32, m: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 11, 9, h, m, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn keys_round_trip_all_variants() {
        for v in [
            DateFormat::PointDMY,
            DateFormat::SlashMMDDYYYY,
            DateFormat::SlashDDMMYYYY,
            DateFormat::HyphenDDMMYYYY,
            DateFormat::HyphenMMDDYYYY,
            DateFormat::HyphenYYYYMMDD,
        ] {
            assert_eq!(v.as_str().parse::<DateFormat>(), Ok(v));
        }
        for v in [TimeFormat::H12, TimeFormat::H24] {
            assert_eq!(v.as_str().parse::<TimeFormat>(), Ok(v));
        }
        for v in [
            IntervalFormat::Decimal,
            IntervalFormat::HoursMinutes,
            IntervalFormat::HoursMinutesColon,
            IntervalFormat::HoursMinutesSecondsColon,
        ] {
            assert_eq!(v.as_str().parse::<IntervalFormat>(), Ok(v));
        }
        for v in [
            NumberFormat::PointComma,
            NumberFormat::CommaPoint,
            NumberFormat::SpaceComma,
            NumberFormat::SpacePoint,
            NumberFormat::ApostrophePoint,
        ] {
            assert_eq!(v.as_str().parse::<NumberFormat>(), Ok(v));
        }
        for v in [
            CurrencyFormat::IsoBeforeSpace,
            CurrencyFormat::IsoAfterSpace,
            CurrencyFormat::SymbolBefore,
            CurrencyFormat::SymbolAfter,
            CurrencyFormat::SymbolBeforeSpace,
            CurrencyFormat::SymbolAfterSpace,
        ] {
            assert_eq!(v.as_str().parse::<CurrencyFormat>(), Ok(v));
        }
        assert!("nope".parse::<DateFormat>().is_err());
    }

    #[test]
    fn solidtime_keys() {
        assert_eq!(DateFormat::PointDMY.as_str(), "point-separated-d-m-yyyy");
        assert_eq!(
            DateFormat::HyphenYYYYMMDD.as_str(),
            "hyphen-separated-yyyy-mm-dd"
        );
        assert_eq!(TimeFormat::H12.as_str(), "12-hours");
        assert_eq!(
            IntervalFormat::HoursMinutesSecondsColon.as_str(),
            "hours-minutes-seconds-colon-separated"
        );
        assert_eq!(NumberFormat::ApostrophePoint.as_str(), "apostrophe-point");
        assert_eq!(
            CurrencyFormat::SymbolAfterSpace.as_str(),
            "symbol-after-with-space"
        );
    }

    #[test]
    fn dates() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(format_date(d, DateFormat::PointDMY), "9.11.2026");
        assert_eq!(format_date(d, DateFormat::SlashMMDDYYYY), "11/09/2026");
        assert_eq!(format_date(d, DateFormat::SlashDDMMYYYY), "09/11/2026");
        assert_eq!(format_date(d, DateFormat::HyphenDDMMYYYY), "09-11-2026");
        assert_eq!(format_date(d, DateFormat::HyphenMMDDYYYY), "11-09-2026");
        assert_eq!(format_date(d, DateFormat::HyphenYYYYMMDD), "2026-11-09");
    }

    #[test]
    fn durations() {
        let mut p = prefs();
        p.number = NumberFormat::CommaPoint;
        p.interval = IntervalFormat::Decimal;
        assert_eq!(format_duration(53106, &p), "14.75 h");
        p.number = NumberFormat::PointComma;
        assert_eq!(format_duration(53106, &p), "14,75 h");
        p.interval = IntervalFormat::HoursMinutes;
        assert_eq!(format_duration(53106, &p), "14h 45min");
        p.interval = IntervalFormat::HoursMinutesColon;
        assert_eq!(format_duration(53106, &p), "14:45");
        assert_eq!(format_duration(65, &p), "0:01");
        p.interval = IntervalFormat::HoursMinutesSecondsColon;
        assert_eq!(format_duration(53106, &p), "14:45:06");
        assert_eq!(format_duration(-5, &p), "0:00:00");
    }

    #[test]
    fn clock() {
        assert_eq!(format_clock(53106), "14:45:06");
        assert_eq!(format_clock(5), "0:00:05");
        assert_eq!(format_clock(-3), "0:00:00");
    }

    #[test]
    fn time_of_day() {
        // 21:05 Moscow (UTC+3) = 18:05 UTC
        let t = utc(18, 5);
        let mut p = prefs();
        p.tz = Moscow;
        p.time = TimeFormat::H24;
        assert_eq!(format_time(t, &p), "21:05");
        p.time = TimeFormat::H12;
        assert_eq!(format_time(t, &p), "9:05 PM");
        assert_eq!(format_time(utc(21, 0), &p), "12:00 AM");
        assert_eq!(format_time(utc(9, 0), &p), "12:00 PM");
    }

    #[test]
    fn numbers() {
        let n = 1234567.891;
        assert_eq!(
            format_number(n, 2, NumberFormat::PointComma),
            "1.234.567,89"
        );
        assert_eq!(
            format_number(n, 2, NumberFormat::CommaPoint),
            "1,234,567.89"
        );
        assert_eq!(
            format_number(n, 2, NumberFormat::SpaceComma),
            "1 234 567,89"
        );
        assert_eq!(
            format_number(n, 2, NumberFormat::SpacePoint),
            "1 234 567.89"
        );
        assert_eq!(
            format_number(n, 2, NumberFormat::ApostrophePoint),
            "1'234'567.89"
        );
        assert_eq!(
            format_number(-1234.5, 1, NumberFormat::CommaPoint),
            "-1,234.5"
        );
        assert_eq!(format_number(2.5, 0, NumberFormat::CommaPoint), "3");
        assert_eq!(format_number(-2.5, 0, NumberFormat::CommaPoint), "-3");
        assert_eq!(format_number(0.999, 2, NumberFormat::CommaPoint), "1.00");
        assert_eq!(format_number(12.0, 0, NumberFormat::CommaPoint), "12");
    }

    #[test]
    fn symbols() {
        assert_eq!(currency_symbol("USD"), "$");
        assert_eq!(currency_symbol("EUR"), "€");
        assert_eq!(currency_symbol("GBP"), "£");
        assert_eq!(currency_symbol("RUB"), "₽");
        assert_eq!(currency_symbol("CNY"), "¥");
        assert_eq!(currency_symbol("JPY"), "¥");
        assert_eq!(currency_symbol("CHF"), "CHF");
    }

    #[test]
    fn money() {
        let mut p = prefs();
        p.currency_code = "EUR".into();
        p.currency = CurrencyFormat::SymbolAfterSpace;
        p.number = NumberFormat::SpaceComma;
        assert_eq!(format_money(123450, &p), "1 234,50 €");
        p.currency_code = "USD".into();
        p.currency = CurrencyFormat::IsoBeforeSpace;
        p.number = NumberFormat::CommaPoint;
        assert_eq!(format_money(123450, &p), "USD 1,234.50");
        p.currency = CurrencyFormat::IsoAfterSpace;
        assert_eq!(format_money(123450, &p), "1,234.50 USD");
        p.currency = CurrencyFormat::SymbolBefore;
        assert_eq!(format_money(123450, &p), "$1,234.50");
        p.currency = CurrencyFormat::SymbolAfter;
        assert_eq!(format_money(123450, &p), "1,234.50$");
        p.currency = CurrencyFormat::SymbolBeforeSpace;
        assert_eq!(format_money(123450, &p), "$ 1,234.50");
        assert_eq!(format_money(-5, &p), "-$ 0.05");
    }

    #[test]
    fn default_prefs() {
        let p = FormatPrefs::default();
        assert_eq!(p.date, DateFormat::HyphenYYYYMMDD);
        assert_eq!(p.time, TimeFormat::H24);
        assert_eq!(p.interval, IntervalFormat::HoursMinutesColon);
        assert_eq!(p.number, NumberFormat::CommaPoint);
        assert_eq!(p.currency, CurrencyFormat::SymbolBefore);
        assert_eq!(p.currency_code, "USD");
        assert_eq!(p.week_start, chrono::Weekday::Mon);
        assert_eq!(p.tz, Tz::UTC);
        assert_eq!(p.hour_suffix, "h");
        assert_eq!(p.minute_suffix, "min");
    }
}
