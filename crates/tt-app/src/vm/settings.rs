//! Settings screen: the choices of each format select with live examples, the mapping
//! between select values and settings, and input validation.

use chrono::{NaiveDate, TimeZone, Utc, Weekday};
use chrono_tz::TZ_VARIANTS;
use tt_domain::{
    CurrencyFormat, DateFormat, FormatPrefs, IntervalFormat, NumberFormat, TimeFormat, Tz,
    format_date, format_duration, format_money, format_number, format_time,
};

use crate::{ProjectOption, SelectOption};

/// The choices of each select, in the order of its options (the option value is the index).
pub const DATES: [DateFormat; 6] = [
    DateFormat::HyphenYYYYMMDD,
    DateFormat::HyphenDDMMYYYY,
    DateFormat::HyphenMMDDYYYY,
    DateFormat::SlashDDMMYYYY,
    DateFormat::SlashMMDDYYYY,
    DateFormat::PointDMY,
];
pub const TIMES: [TimeFormat; 2] = [TimeFormat::H24, TimeFormat::H12];
pub const INTERVALS: [IntervalFormat; 4] = [
    IntervalFormat::HoursMinutesColon,
    IntervalFormat::HoursMinutesSecondsColon,
    IntervalFormat::HoursMinutes,
    IntervalFormat::Decimal,
];
pub const NUMBERS: [NumberFormat; 5] = [
    NumberFormat::CommaPoint,
    NumberFormat::PointComma,
    NumberFormat::SpaceComma,
    NumberFormat::SpacePoint,
    NumberFormat::ApostrophePoint,
];
pub const CURRENCIES: [CurrencyFormat; 6] = [
    CurrencyFormat::SymbolBefore,
    CurrencyFormat::SymbolAfter,
    CurrencyFormat::SymbolBeforeSpace,
    CurrencyFormat::SymbolAfterSpace,
    CurrencyFormat::IsoBeforeSpace,
    CurrencyFormat::IsoAfterSpace,
];
pub const WEEK_STARTS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

/// Select value of `v` in `list`; 0 (the first choice) if it is not there.
pub fn index_of<T: PartialEq>(list: &[T], v: &T) -> i32 {
    list.iter().position(|x| x == v).unwrap_or(0) as i32
}

/// The choice with select value `i`.
pub fn choice<T: Copy>(list: &[T], i: i32) -> Option<T> {
    usize::try_from(i).ok().and_then(|i| list.get(i)).copied()
}

/// Select options whose values are the positions of `labels`.
pub fn select_options(labels: Vec<String>) -> Vec<SelectOption> {
    labels
        .into_iter()
        .enumerate()
        .map(|(i, label)| SelectOption {
            value: i as i32,
            label: label.into(),
        })
        .collect()
}

/// Example texts of every choice (same order as the constants), formatted with the other
/// current preferences; they are what the format selects show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Examples {
    pub dates: Vec<String>,
    pub times: Vec<String>,
    pub intervals: Vec<String>,
    pub numbers: Vec<String>,
    pub currencies: Vec<String>,
}

/// Samples: 2026-11-09 13:45:30 UTC, 1 h 30 min 45 s, 1234567.89 and 1234.56.
pub fn examples(prefs: &FormatPrefs) -> Examples {
    let date = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap_or_default();
    let at = Utc
        .with_ymd_and_hms(2026, 11, 9, 13, 45, 30)
        .single()
        .map_or(0, |t| t.timestamp_millis());
    // The user's preferences with one of them replaced (time shown in UTC).
    let with = |f: &dyn Fn(&mut FormatPrefs)| {
        let mut p = FormatPrefs {
            tz: Tz::UTC,
            ..prefs.clone()
        };
        f(&mut p);
        p
    };
    Examples {
        dates: DATES.iter().map(|f| format_date(date, *f)).collect(),
        times: TIMES
            .iter()
            .map(|f| format_time(at, &with(&|p| p.time = *f)))
            .collect(),
        intervals: INTERVALS
            .iter()
            .map(|f| format_duration(5445, &with(&|p| p.interval = *f)))
            .collect(),
        numbers: NUMBERS
            .iter()
            .map(|f| format_number(1_234_567.89, 2, *f))
            .collect(),
        currencies: CURRENCIES
            .iter()
            .map(|f| format_money(123_456, &with(&|p| p.currency = *f)))
            .collect(),
    }
}

/// A currency code as typed: three ASCII letters, returned in capitals.
pub fn valid_currency_code(s: &str) -> Option<String> {
    let s = s.trim();
    (s.len() == 3 && s.bytes().all(|b| b.is_ascii_alphabetic())).then(|| s.to_ascii_uppercase())
}

/// Every IANA time zone as a picker option (id and label = the name).
pub fn timezone_options() -> Vec<ProjectOption> {
    TZ_VARIANTS
        .iter()
        .map(|tz| ProjectOption {
            id: tz.name().into(),
            label: tz.name().into(),
            ..ProjectOption::default()
        })
        .collect()
}

/// Transparency of the idle mini timer as the slider has it: 0 to 90 in steps of 5.
pub fn clamp_transparency(percent: i32) -> i32 {
    (percent.clamp(0, 90) + 2) / 5 * 5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> FormatPrefs {
        FormatPrefs::default()
    }

    #[test]
    fn examples_show_every_choice_with_the_sample_values() {
        let e = examples(&en());
        assert_eq!(e.dates.len(), DATES.len());
        assert_eq!(e.dates[0], "2026-11-09");
        assert_eq!(e.dates[3], "09/11/2026");
        assert_eq!(e.dates[5], "9.11.2026");
        assert_eq!(e.times, ["13:45", "1:45 PM"]);
        assert_eq!(e.intervals, ["1:30", "1:30:45", "1h 30min", "1.51 h"]);
        assert_eq!(e.numbers[0], "1,234,567.89");
        assert_eq!(e.numbers[1], "1.234.567,89");
        assert_eq!(e.numbers[4], "1'234'567.89");
        assert_eq!(e.currencies[0], "$1,234.56");
        assert_eq!(e.currencies[5], "1,234.56 USD");
    }

    #[test]
    fn examples_follow_the_other_preferences() {
        let mut p = en();
        p.number = NumberFormat::SpaceComma;
        p.currency_code = "EUR".into();
        p.hour_suffix = "ч".into();
        p.minute_suffix = "мин".into();
        let e = examples(&p);
        assert_eq!(e.currencies[1], "1 234,56€");
        assert_eq!(e.intervals[2], "1ч 30мин");
        assert_eq!(e.intervals[3], "1,51 ч");
        // The user's zone does not shift the sample time.
        p.tz = Tz::Asia__Tokyo;
        assert_eq!(examples(&p).times[0], "13:45");
    }

    #[test]
    fn choices_cover_all_variants_and_map_back() {
        assert_eq!(index_of(&DATES, &DateFormat::PointDMY), 5);
        assert_eq!(index_of(&WEEK_STARTS, &Weekday::Sun), 6);
        assert_eq!(index_of(&NUMBERS, &NumberFormat::SpacePoint), 3);
        assert_eq!(choice(&CURRENCIES, 4), Some(CurrencyFormat::IsoBeforeSpace));
        assert_eq!(choice(&TIMES, 2), None);
        assert_eq!(choice(&TIMES, -1), None);
        // The default format is the first choice (and every choice is distinct).
        let d = FormatPrefs::default();
        assert_eq!(index_of(&DATES, &d.date), 0);
        assert_eq!(index_of(&INTERVALS, &d.interval), 0);
        assert_eq!(index_of(&NUMBERS, &d.number), 0);
        let mut dates = DATES.map(DateFormat::as_str).to_vec();
        dates.sort_unstable();
        dates.dedup();
        assert_eq!(dates.len(), DATES.len());
        let options = select_options(vec!["a".into(), "b".into()]);
        assert_eq!((options[1].value, options[1].label.as_str()), (1, "b"));
    }

    #[test]
    fn currency_code_is_three_letters_in_capitals() {
        assert_eq!(valid_currency_code("usd").as_deref(), Some("USD"));
        assert_eq!(valid_currency_code(" eUr ").as_deref(), Some("EUR"));
        for bad in ["", "US", "USDX", "U5D", "€€€", "ЕUR", "us d"] {
            assert_eq!(valid_currency_code(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn timezone_list_has_the_iana_names() {
        let zones = timezone_options();
        assert!(zones.len() > 400);
        let find = |n: &str| zones.iter().find(|z| z.label == n).map(|z| z.id.clone());
        assert_eq!(find("Europe/Berlin").as_deref(), Some("Europe/Berlin"));
        assert!(find("UTC").is_some());
        assert!(zones.iter().all(|z| z.id.parse::<Tz>().is_ok()));
    }

    #[test]
    fn transparency_is_clamped_to_0_90_in_steps_of_5() {
        assert_eq!(clamp_transparency(70), 70);
        assert_eq!(clamp_transparency(-5), 0);
        assert_eq!(clamp_transparency(100), 90);
        assert_eq!(clamp_transparency(72), 70);
        assert_eq!(clamp_transparency(73), 75);
    }
}
