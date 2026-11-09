//! Time screen: day groups and rows, the entry dialog, and parsing of typed times and
//! durations.

use chrono::{DateTime, Days, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use slint::{Model, ModelRc, SharedString, VecModel};
use tt_domain::{
    DateFormat, EntryTemplate, FormatPrefs, Id, Millis, TimeEntry, TimeFormat, Tz, format_date,
    format_duration, format_time, local_date,
};

use crate::vm::dates::{iso, parse_iso};
use crate::vm::pickers::{Catalog, hex_color, option_label, parse_id};
use crate::{EntryDialogData, EntryDialogView, EntryRowView};

/// A day as returned by `EntryService::list_days`: date, entries newest first, total ms.
pub type Day = (NaiveDate, Vec<TimeEntry>, i64);

const MINUTE_MS: i64 = 60_000;
const DAY_S: i64 = 24 * 3600;
/// Longest accepted duration input (seconds); larger numbers are typos.
const MAX_DURATION_S: f64 = 1e9;
/// Between start and end in a row's range.
pub const RANGE_DASH: &str = " – ";
/// End of a running entry's range.
pub const RUNNING_END: &str = "…";

/// Unit words accepted after a number in a duration ("1h 30m", "1ч 30мин", "1 hora").
const HOUR_UNITS: [&str; 13] = [
    "h",
    "hr",
    "hrs",
    "hour",
    "hours",
    "ч",
    "час",
    "часа",
    "часов",
    "时",
    "小时",
    "hora",
    "horas",
];
const MINUTE_UNITS: [&str; 13] = [
    "m",
    "min",
    "mins",
    "minute",
    "minutes",
    "м",
    "мин",
    "минут",
    "минуты",
    "分",
    "分钟",
    "minuto",
    "minutos",
];

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Two ASCII digits below 60.
fn minutes_part(s: &str) -> Option<u32> {
    if s.len() != 2 || !all_digits(s) {
        return None;
    }
    s.parse().ok().filter(|m| *m < 60)
}

/// Time of day typed by the user: "9:05", "0905", "905", "9", "21:30"; with "am" / "pm"
/// ("9pm", "9:30 PM") as shown in the 12-hour format. `None` for anything else.
pub fn parse_time_input(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (body, half) = match (s.strip_suffix("am"), s.strip_suffix("pm")) {
        (Some(b), _) => (b.trim_end(), Some(false)),
        (_, Some(b)) => (b.trim_end(), Some(true)),
        _ => (s.as_str(), None),
    };
    let (h, m) = match body.split_once([':', '.']) {
        Some((h, m)) if (1..=2).contains(&h.len()) && all_digits(h) => {
            (h.parse::<u32>().ok()?, minutes_part(m)?)
        }
        Some(_) => return None,
        None if !all_digits(body) => return None,
        None => match body.len() {
            1 | 2 => (body.parse().ok()?, 0),
            3 => (body[..1].parse().ok()?, minutes_part(&body[1..])?),
            4 => (body[..2].parse().ok()?, minutes_part(&body[2..])?),
            _ => return None,
        },
    };
    match half {
        None if h < 24 => Some((h, m)),
        Some(pm) if (1..=12).contains(&h) => Some((h % 12 + if pm { 12 } else { 0 }, m)),
        _ => None,
    }
}

/// "1.5" / "1,5" / "2": digits with at most one decimal separator.
fn parse_number(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let digits_ok = (int.is_empty() || all_digits(int)) && (frac.is_empty() || all_digits(frac));
    if !digits_ok || (int.is_empty() && frac.is_empty()) {
        return None;
    }
    s.parse().ok()
}

/// "1h 30m", "90 min", "1,5h": numbers each followed by a unit, hours before minutes.
fn parse_with_units(s: &str) -> Option<f64> {
    let mut rest = s;
    let mut parts: Vec<(f64, bool)> = Vec::new();
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
            .unwrap_or(rest.len());
        let n = parse_number(&rest[..num_len])?;
        rest = rest[num_len..].trim_start();
        let unit_len = rest
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(rest.len());
        let unit = &rest[..unit_len];
        let is_hour = if HOUR_UNITS.contains(&unit) {
            true
        } else if MINUTE_UNITS.contains(&unit) {
            false
        } else {
            return None;
        };
        parts.push((n, is_hour));
        rest = rest[unit_len..].trim_start();
    }
    match parts.as_slice() {
        [(h, true)] => Some(h * 3600.0),
        [(m, false)] => Some(m * 60.0),
        [(h, true), (m, false)] => Some(h * 3600.0 + m * 60.0),
        _ => None,
    }
}

/// "H:MM" or "H:MM:SS".
fn parse_colon(s: &str) -> Option<f64> {
    let mut it = s.split(':');
    let h = it.next().filter(|h| all_digits(h) && h.len() <= 9)?;
    let m = minutes_part(it.next()?)?;
    let sec = match it.next() {
        Some(x) => minutes_part(x)?,
        None => 0,
    };
    if it.next().is_some() {
        return None;
    }
    Some(h.parse::<f64>().ok()? * 3600.0 + f64::from(m * 60 + sec))
}

/// Duration typed by the user in seconds: "1:30", "1.5", "1,5", "90m", "1h 30m", "2"
/// (hours). `None` for anything else.
pub fn parse_duration_input(s: &str) -> Option<i64> {
    let s = s.trim().to_lowercase();
    let seconds = if s.contains(':') {
        parse_colon(&s)?
    } else if let Some(h) = parse_number(&s) {
        h * 3600.0
    } else {
        parse_with_units(&s)?
    };
    (seconds.is_finite() && seconds <= MAX_DURATION_S).then(|| seconds.round() as i64)
}

/// "Today", "Yesterday" or the date in the user's format.
pub fn day_title(
    d: NaiveDate,
    today: NaiveDate,
    f: DateFormat,
    today_text: &str,
    yesterday_text: &str,
) -> String {
    if d == today {
        today_text.to_owned()
    } else if today.pred_opt() == Some(d) {
        yesterday_text.to_owned()
    } else {
        format_date(d, f)
    }
}

/// Sum of the entries; a running entry counts up to `now`.
pub fn day_total_ms(entries: &[TimeEntry], now: Millis) -> i64 {
    entries.iter().map(|e| e.duration_ms(now)).sum()
}

/// "09:30 – 10:15", running "09:30 – …".
pub fn range_text(e: &TimeEntry, prefs: &FormatPrefs) -> String {
    let end = e
        .end
        .map_or_else(|| RUNNING_END.to_owned(), |t| format_time(t, prefs));
    format!("{}{RANGE_DASH}{end}", format_time(e.start, prefs))
}

/// Tag names of `ids` in catalog order, joined with ", ".
fn tag_names(c: &Catalog, ids: &[Id]) -> String {
    let names: Vec<&str> = c
        .tags
        .iter()
        .filter(|t| ids.contains(&t.id))
        .map(|t| t.name.as_str())
        .collect();
    names.join(", ")
}

/// "Project › Task" and the project color, or empty / transparent.
fn project_label(e: &TimeEntry, c: &Catalog) -> (String, slint::Color) {
    let Some(p) = e.project_id.and_then(|id| c.project(id)) else {
        return (String::new(), slint::Color::default());
    };
    let task = e.task_id.and_then(|id| c.task(id));
    (option_label(p, task), hex_color(&p.color))
}

pub fn entry_row(e: &TimeEntry, c: &Catalog, prefs: &FormatPrefs, now: Millis) -> EntryRowView {
    let (label, color) = project_label(e, c);
    EntryRowView {
        id: e.id.to_string().into(),
        description: e.description.as_str().into(),
        project_label: label.into(),
        project_color: color,
        tags: tag_names(c, &e.tag_ids).into(),
        billable: e.billable,
        range: range_text(e, prefs).into(),
        duration: format_duration(e.duration_ms(now) / 1000, prefs).into(),
        running: e.end.is_none(),
    }
}

/// Time of day as shown: "09:05" or "9:05 PM" (same as `format_time`).
pub fn time_text(h: u32, m: u32, f: TimeFormat) -> String {
    match f {
        TimeFormat::H24 => format!("{h:02}:{m:02}"),
        TimeFormat::H12 => {
            let suffix = if h < 12 { "AM" } else { "PM" };
            let h12 = (h + 11) % 12 + 1;
            format!("{h12}:{m:02} {suffix}")
        }
    }
}

/// Duration as typed in the dialog: "1:30".
pub fn hm_text(seconds: i64) -> String {
    let s = seconds.max(0);
    format!("{}:{:02}", s / 3600, s % 3600 / 60)
}

/// The typed time in display form, "" if not parseable.
pub fn normalize_time(s: &str, f: TimeFormat) -> String {
    parse_time_input(s).map_or_else(String::new, |(h, m)| time_text(h, m, f))
}

/// A typed duration of less than 24 hours.
fn day_duration(s: &str) -> Option<i64> {
    parse_duration_input(s).filter(|d| (0..DAY_S).contains(d))
}

/// The typed duration as "H:MM", "" unless it is less than 24 h. Zero is accepted: timer
/// entries under a minute show as "0:00", and the save rejects an empty range.
pub fn normalize_duration(s: &str) -> String {
    day_duration(s).map_or_else(String::new, hm_text)
}

fn minute_of_day(s: &str) -> Option<i64> {
    parse_time_input(s).map(|(h, m)| i64::from(h * 60 + m))
}

/// "H:MM" from `start` to `end` (an end not after the start is on the next day).
pub fn duration_between(start: &str, end: &str) -> String {
    match (minute_of_day(start), minute_of_day(end)) {
        (Some(s), Some(e)) => hm_text((e - s).rem_euclid(24 * 60) * 60),
        _ => String::new(),
    }
}

/// End time of day of `start` + `duration`, "" if either is invalid.
pub fn end_after(start: &str, duration: &str, f: TimeFormat) -> String {
    match (minute_of_day(start), day_duration(duration)) {
        (Some(s), Some(d)) => {
            let end = (s + d / 60).rem_euclid(24 * 60);
            time_text((end / 60) as u32, (end % 60) as u32, f)
        }
        _ => String::new(),
    }
}

/// Instant of local `date` at `h:m` in `tz`; a time skipped by DST moves forward.
pub fn local_millis(date: NaiveDate, h: u32, m: u32, tz: Tz) -> Millis {
    let Some(mut naive) = date.and_hms_opt(h, m, 0) else {
        return 0;
    };
    // At most a few hours are skipped.
    for _ in 0..24 * 4 {
        if let Some(dt) = tz.from_local_datetime(&naive).earliest() {
            return dt.timestamp_millis();
        }
        naive += chrono::Duration::minutes(15);
    }
    0
}

/// Local wall times (to the minute) of the dialog fields; the end is on the next day when
/// it is not after the start, `None` for an empty end.
fn dialog_wall(
    date: &str,
    start: &str,
    end: &str,
) -> Option<(NaiveDateTime, Option<NaiveDateTime>)> {
    let day = parse_iso(date)?;
    let (sh, sm) = parse_time_input(start)?;
    let s = day.and_hms_opt(sh, sm, 0)?;
    if end.trim().is_empty() {
        return Some((s, None));
    }
    let (eh, em) = parse_time_input(end)?;
    let end_day = if (eh, em) < (sh, sm) {
        day.checked_add_days(Days::new(1))?
    } else {
        day
    };
    Some((s, Some(end_day.and_hms_opt(eh, em, 0)?)))
}

/// Local wall time of `t` truncated to the minute.
fn wall_minute(t: Millis, tz: Tz) -> Option<NaiveDateTime> {
    let local = DateTime::<Utc>::from_timestamp_millis(t)?
        .with_timezone(&tz)
        .naive_local();
    local.with_second(0)?.with_nanosecond(0)
}

/// `original` if it shows as `wall`, else the instant of `wall`.
fn instant(wall: NaiveDateTime, tz: Tz, original: Option<Millis>) -> Millis {
    match original {
        Some(t) if wall_minute(t, tz) == Some(wall) => t,
        _ => local_millis(wall.date(), wall.hour(), wall.minute(), tz),
    }
}

/// Start and end (`None` for an empty end = running) of the dialog fields; the end is on
/// the next day when it is not after the start. `None` if a field is invalid.
pub fn dialog_range(
    date: &str,
    start: &str,
    end: &str,
    tz: Tz,
) -> Option<(Millis, Option<Millis>)> {
    dialog_range_from(date, start, end, tz, None)
}

/// Like [`dialog_range`], but a field that still shows the `original` start / end (same
/// local date and time to the minute) keeps the original instant, seconds included, so
/// saving an otherwise edited entry does not move it.
pub fn dialog_range_from(
    date: &str,
    start: &str,
    end: &str,
    tz: Tz,
    original: Option<(Millis, Option<Millis>)>,
) -> Option<(Millis, Option<Millis>)> {
    let (s, e) = dialog_wall(date, start, end)?;
    let (orig_start, orig_end) = original.map_or((None, None), |(s, e)| (Some(s), e));
    Some((
        instant(s, tz, orig_start),
        e.map(|e| instant(e, tz, orig_end)),
    ))
}

pub fn template_of(d: &EntryDialogData) -> EntryTemplate {
    let project_id = parse_id(&d.project_id);
    EntryTemplate {
        description: d.description.to_string(),
        project_id,
        task_id: project_id.and(parse_id(&d.task_id)),
        tag_ids: d.tag_ids.iter().filter_map(|s| parse_id(&s)).collect(),
        billable: d.billable,
    }
}

fn id_text(id: Option<Id>) -> SharedString {
    id.map(|id| id.to_string().into()).unwrap_or_default()
}

fn local_time_text(t: Millis, prefs: &FormatPrefs) -> SharedString {
    format_time(t, prefs).into()
}

/// Dialog contents for editing `e`.
pub fn edit_view(e: &TimeEntry, c: &Catalog, prefs: &FormatPrefs, now: Millis) -> EntryDialogView {
    let (label, color) = project_label(e, c);
    let tag_ids: Vec<SharedString> = e.tag_ids.iter().map(|id| id.to_string().into()).collect();
    EntryDialogView {
        data: EntryDialogData {
            id: e.id.to_string().into(),
            description: e.description.as_str().into(),
            project_id: id_text(e.project_id),
            task_id: id_text(e.project_id.and(e.task_id)),
            tag_ids: ModelRc::new(VecModel::from(tag_ids)),
            billable: e.billable,
            date: iso(local_date(e.start, prefs.tz)),
            start: local_time_text(e.start, prefs),
            end: e.end.map(|t| local_time_text(t, prefs)).unwrap_or_default(),
        },
        running: e.end.is_none(),
        project_label: label.into(),
        project_color: color,
        duration: hm_text(e.duration_ms(now) / 1000).into(),
    }
}

/// Dialog contents for a new manual entry: the hour before `now` rounded down to 5 min.
pub fn new_entry_view(now: Millis, prefs: &FormatPrefs) -> EntryDialogView {
    const STEP: i64 = 5 * MINUTE_MS;
    let end = now.div_euclid(STEP) * STEP;
    let start = end - 60 * MINUTE_MS;
    EntryDialogView {
        data: EntryDialogData {
            date: iso(local_date(start, prefs.tz)),
            start: local_time_text(start, prefs),
            end: local_time_text(end, prefs),
            ..Default::default()
        },
        duration: hm_text(3600).into(),
        ..Default::default()
    }
}

/// The bound after loading `page` (asked for `batch` days): its oldest date, or `None`
/// when the page is short (nothing older exists).
pub fn page_bound(page: &[Day], batch: usize) -> Option<NaiveDate> {
    if page.len() < batch {
        return None;
    }
    page.last().map(|d| d.0)
}

/// Reloads the days from today back to `since` (inclusive), or all days for `None`, with
/// `fetch(before)` returning up to `batch` days before `before`. Returns the days and the
/// oldest date still to keep on the next reload (`None` = everything is loaded).
pub fn reload_days<E>(
    since: Option<NaiveDate>,
    batch: usize,
    mut fetch: impl FnMut(Option<NaiveDate>) -> Result<Vec<Day>, E>,
) -> Result<(Vec<Day>, Option<NaiveDate>), E> {
    let mut out: Vec<Day> = Vec::new();
    let mut before = None;
    loop {
        let page = fetch(before)?;
        let bound = page_bound(&page, batch);
        out.extend(page);
        let reached = matches!((since, bound), (Some(s), Some(b)) if b <= s);
        if bound.is_none() || reached {
            let Some(s) = since else {
                return Ok((out, None));
            };
            let n = out.len();
            out.retain(|d| d.0 >= s);
            // Complete only if the last page ended and nothing older was dropped.
            let complete = bound.is_none() && out.len() == n;
            return Ok((out, if complete { None } else { Some(s) }));
        }
        before = bound;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use slint::Model;
    use tt_domain::{IntervalFormat, new_id};

    use crate::vm::pickers::tests::{project, tag, task};

    const MSK: Tz = Tz::Europe__Moscow;
    const HOUR_MS: i64 = 60 * MINUTE_MS;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Moscow wall-clock time in November 2026.
    fn at(d: u32, h: u32, mi: u32) -> Millis {
        MSK.with_ymd_and_hms(2026, 11, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn prefs() -> FormatPrefs {
        FormatPrefs {
            interval: IntervalFormat::HoursMinutesColon,
            tz: MSK,
            ..FormatPrefs::default()
        }
    }

    fn entry(start: Millis, end: Option<Millis>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: "Review".into(),
            start,
            end,
            billable: false,
            project_id: None,
            task_id: None,
            tag_ids: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn time_input_examples() {
        let ok = [
            ("9:05", (9, 5)),
            ("0905", (9, 5)),
            ("905", (9, 5)),
            ("9", (9, 0)),
            ("21:30", (21, 30)),
            ("21", (21, 0)),
            ("00:00", (0, 0)),
            ("23:59", (23, 59)),
            (" 9:05 ", (9, 5)),
            ("9.05", (9, 5)),
            ("9pm", (21, 0)),
            ("9:30 PM", (21, 30)),
            ("12am", (0, 0)),
            ("12:15 am", (0, 15)),
            ("12pm", (12, 0)),
        ];
        for (s, want) in ok {
            assert_eq!(parse_time_input(s), Some(want), "{s:?}");
        }
    }

    #[test]
    fn time_input_garbage_is_none() {
        for s in [
            "", " ", "abc", "24:00", "9:60", "25", "12345", "9:5", ":30", "9:", "9::30", "-9",
            "13pm", "0am", "9 xm", "9:05:00", "٩:٠٥", "9h",
        ] {
            assert_eq!(parse_time_input(s), None, "{s:?}");
        }
    }

    #[test]
    fn duration_input_examples() {
        for s in [
            "1:30",
            "1.5",
            "1,5",
            "90m",
            "1h 30m",
            "1h30m",
            "1ч 30мин",
            "90 min",
        ] {
            assert_eq!(parse_duration_input(s), Some(5400), "{s:?}");
        }
        assert_eq!(parse_duration_input("2"), Some(7200));
        assert_eq!(parse_duration_input("2h"), Some(7200));
        assert_eq!(parse_duration_input("0:05"), Some(300));
        assert_eq!(parse_duration_input("1:30:15"), Some(5415));
        assert_eq!(parse_duration_input("0.25"), Some(900));
        assert_eq!(parse_duration_input(" 45m "), Some(2700));
        assert_eq!(parse_duration_input("1,5h 2m"), Some(5520));
        assert_eq!(parse_duration_input("0"), Some(0));
    }

    #[test]
    fn duration_input_garbage_is_none() {
        for s in [
            "",
            "abc",
            "1:75",
            "1:3",
            "-1",
            "1h 30",
            "h",
            "1.5.5",
            "m30",
            "1:30:75",
            "1hh",
            "30m 1h",
            "99999999999999999999h",
            "1h 2h",
            "1.5x",
        ] {
            assert_eq!(parse_duration_input(s), None, "{s:?}");
        }
    }

    #[test]
    fn day_title_today_yesterday_or_date() {
        let today = date(2026, 11, 9);
        let t = |d| day_title(d, today, DateFormat::PointDMY, "Today", "Yesterday");
        assert_eq!(t(today), "Today");
        assert_eq!(t(date(2026, 11, 8)), "Yesterday");
        assert_eq!(t(date(2026, 11, 7)), "7.11.2026");
        assert_eq!(t(date(2025, 11, 9)), "9.11.2025");
        assert_eq!(
            day_title(
                date(2026, 11, 1),
                today,
                DateFormat::HyphenYYYYMMDD,
                "a",
                "b"
            ),
            "2026-11-01"
        );
    }

    #[test]
    fn day_total_counts_running_entry_up_to_now() {
        let list = [
            entry(at(9, 12, 0), None),
            entry(at(9, 10, 0), Some(at(9, 11, 30))),
            entry(at(9, 9, 0), Some(at(9, 9, 15))),
        ];
        let now = at(9, 12, 20);
        assert_eq!(day_total_ms(&list, now), 20 * MINUTE_MS + 105 * MINUTE_MS);
        assert_eq!(day_total_ms(&[], now), 0);
    }

    #[test]
    fn row_shows_range_duration_project_and_tags() {
        let web = project("Website", "#8b5cf6");
        let reports = task(&web, "Reports");
        let (design, backend) = (tag("design"), tag("backend"));
        let c = Catalog {
            projects: vec![web.clone()],
            tasks: vec![reports.clone()],
            tags: vec![backend.clone(), design.clone()],
        };
        let mut e = entry(at(9, 9, 30), Some(at(9, 10, 15)));
        e.project_id = Some(web.id);
        e.task_id = Some(reports.id);
        e.tag_ids = vec![design.id, backend.id];
        e.billable = true;
        let r = entry_row(&e, &c, &prefs(), at(9, 12, 0));
        assert_eq!(r.id, e.id.to_string());
        assert_eq!(r.description, "Review");
        assert_eq!(r.project_label, "Website › Reports");
        assert_eq!(r.project_color, hex_color("#8b5cf6"));
        assert_eq!(r.tags, "backend, design");
        assert!(r.billable);
        assert_eq!(r.range, "09:30 – 10:15");
        assert_eq!(r.duration, "0:45");
        assert!(!r.running);
    }

    #[test]
    fn running_row_counts_up_to_now() {
        let e = entry(at(9, 9, 30), None);
        let mut p = prefs();
        p.time = TimeFormat::H12;
        let r = entry_row(&e, &Catalog::default(), &p, at(9, 11, 0));
        assert_eq!(r.range, "9:30 AM – …");
        assert_eq!(r.duration, "1:30");
        assert!(r.running);
        assert_eq!(r.project_label, "");
        assert_eq!(r.tags, "");
    }

    #[test]
    fn time_and_duration_texts() {
        assert_eq!(time_text(9, 5, TimeFormat::H24), "09:05");
        assert_eq!(time_text(21, 5, TimeFormat::H12), "9:05 PM");
        assert_eq!(time_text(0, 0, TimeFormat::H12), "12:00 AM");
        assert_eq!(hm_text(5400), "1:30");
        assert_eq!(hm_text(59), "0:00");
        assert_eq!(normalize_time("905", TimeFormat::H24), "09:05");
        assert_eq!(normalize_time("9pm", TimeFormat::H24), "21:00");
        assert_eq!(normalize_time("21:30", TimeFormat::H12), "9:30 PM");
        assert_eq!(normalize_time("x", TimeFormat::H24), "");
        assert_eq!(normalize_duration("90m"), "1:30");
        assert_eq!(normalize_duration("0"), "0:00");
        assert_eq!(normalize_duration("24h"), "");
        assert_eq!(normalize_duration("23:59"), "23:59");
    }

    #[test]
    fn duration_and_end_follow_each_other() {
        assert_eq!(duration_between("9:30", "10:15"), "0:45");
        // Across midnight.
        assert_eq!(duration_between("23:00", "1:30"), "2:30");
        assert_eq!(duration_between("9:30", "9:30"), "0:00");
        assert_eq!(duration_between("9:30", "x"), "");
        assert_eq!(end_after("9:30", "1h 30m", TimeFormat::H24), "11:00");
        assert_eq!(end_after("23:00", "2:30", TimeFormat::H24), "01:30");
        assert_eq!(end_after("9:30", "45m", TimeFormat::H12), "10:15 AM");
        assert_eq!(end_after("9:30", "24h", TimeFormat::H24), "");
        assert_eq!(end_after("9:30", "0", TimeFormat::H24), "09:30");
        assert_eq!(end_after("x", "1h", TimeFormat::H24), "");
    }

    #[test]
    fn dialog_range_puts_an_earlier_end_on_the_next_day() {
        assert_eq!(
            dialog_range("2026-11-09", "9:30", "10:15", MSK),
            Some((at(9, 9, 30), Some(at(9, 10, 15))))
        );
        assert_eq!(
            dialog_range("2026-11-09", "23:00", "1:00", MSK),
            Some((at(9, 23, 0), Some(at(10, 1, 0))))
        );
        // Equal: zero length (rejected by the service).
        assert_eq!(
            dialog_range("2026-11-09", "9:30", "9:30", MSK),
            Some((at(9, 9, 30), Some(at(9, 9, 30))))
        );
        // Running: no end.
        assert_eq!(
            dialog_range("2026-11-09", "9:30", "", MSK),
            Some((at(9, 9, 30), None))
        );
        assert_eq!(dialog_range("2026-11-9x", "9:30", "10:00", MSK), None);
        assert_eq!(dialog_range("2026-11-09", "9:3", "10:00", MSK), None);
        assert_eq!(dialog_range("2026-11-09", "9:30", "1o:00", MSK), None);
    }

    #[test]
    fn local_millis_moves_forward_over_dst_gap() {
        // America/Sao_Paulo 2018-11-04: 00:00 → 01:00.
        let tz = Tz::America__Sao_Paulo;
        let d = date(2018, 11, 4);
        let one = tz.with_ymd_and_hms(2018, 11, 4, 1, 0, 0).unwrap();
        assert_eq!(local_millis(d, 0, 30, tz), one.timestamp_millis());
        assert_eq!(local_millis(d, 1, 0, tz), one.timestamp_millis());
        assert_eq!(local_millis(date(2026, 11, 9), 9, 30, MSK), at(9, 9, 30));
    }

    #[test]
    fn edit_view_fills_dialog_from_entry() {
        let web = project("Website", "#8b5cf6");
        let reports = task(&web, "Reports");
        let urgent = tag("urgent");
        let c = Catalog {
            projects: vec![web.clone()],
            tasks: vec![reports.clone()],
            tags: vec![urgent.clone()],
        };
        let mut e = entry(at(9, 23, 0), Some(at(10, 0, 0)));
        e.project_id = Some(web.id);
        e.task_id = Some(reports.id);
        e.tag_ids = vec![urgent.id];
        e.billable = true;
        let v = edit_view(&e, &c, &prefs(), at(10, 9, 0));
        assert_eq!(v.data.id, e.id.to_string());
        assert_eq!(v.data.description, "Review");
        assert_eq!(v.data.project_id, web.id.to_string());
        assert_eq!(v.data.task_id, reports.id.to_string());
        let tags: Vec<SharedString> = v.data.tag_ids.iter().collect();
        assert_eq!(tags, vec![SharedString::from(urgent.id.to_string())]);
        assert!(v.data.billable);
        assert_eq!(v.data.date, "2026-11-09");
        assert_eq!(v.data.start, "23:00");
        // Ends at the next midnight.
        assert_eq!(v.data.end, "00:00");
        assert_eq!(v.duration, "1:00");
        assert!(!v.running);
        assert_eq!(v.project_label, "Website › Reports");
        assert_eq!(v.project_color, hex_color("#8b5cf6"));
        // Saving the unchanged fields gives the same range.
        assert_eq!(
            dialog_range(&v.data.date, &v.data.start, &v.data.end, MSK),
            Some((e.start, e.end))
        );
        assert_eq!(template_of(&v.data), EntryTemplate::from_entry(&e));

        let running = entry(at(10, 8, 0), None);
        let v = edit_view(&running, &c, &prefs(), at(10, 9, 15));
        assert!(v.running);
        assert_eq!(v.data.end, "");
        assert_eq!(v.duration, "1:15");
        assert_eq!(v.project_label, "");
    }

    #[test]
    fn new_entry_is_the_hour_before_now() {
        let v = new_entry_view(at(9, 14, 47) + 12_345, &prefs());
        assert_eq!(v.data.id, "");
        assert_eq!(v.data.date, "2026-11-09");
        assert_eq!(v.data.start, "13:45");
        assert_eq!(v.data.end, "14:45");
        assert_eq!(v.duration, "1:00");
        assert!(!v.data.billable && !v.running);
        // Shortly after midnight the start is yesterday.
        let v = new_entry_view(at(9, 0, 20), &prefs());
        assert_eq!(
            (
                v.data.date.as_str(),
                v.data.start.as_str(),
                v.data.end.as_str()
            ),
            ("2026-11-08", "23:20", "00:20")
        );
        assert_eq!(
            dialog_range(&v.data.date, &v.data.start, &v.data.end, MSK),
            Some((at(8, 23, 20), Some(at(9, 0, 20))))
        );
    }

    #[test]
    fn template_parses_ids() {
        let pid = new_id();
        let d = EntryDialogData {
            description: "x".into(),
            project_id: pid.to_string().into(),
            task_id: "".into(),
            tag_ids: ModelRc::new(VecModel::from(vec![SharedString::from("bad")])),
            billable: true,
            ..Default::default()
        };
        let t = template_of(&d);
        assert_eq!(t.description, "x");
        assert_eq!(t.project_id, Some(pid));
        assert_eq!(t.task_id, None);
        assert!(t.tag_ids.is_empty());
        assert!(t.billable);
    }

    /// An entry with seconds and milliseconds, as the timer writes them.
    fn precise() -> TimeEntry {
        entry(at(9, 9, 30) + 12_345, Some(at(9, 10, 15) + 45_678))
    }

    fn range_of(v: &EntryDialogView, e: &TimeEntry) -> Option<(Millis, Option<Millis>)> {
        let d = &v.data;
        dialog_range_from(&d.date, &d.start, &d.end, MSK, Some((e.start, e.end)))
    }

    #[test]
    fn unchanged_fields_keep_exact_instants() {
        let e = precise();
        let v = edit_view(&e, &Catalog::default(), &prefs(), at(9, 12, 0));
        assert_eq!(range_of(&v, &e), Some((e.start, e.end)));
        // Typed differently but the same time: still the original instant.
        let mut v2 = v.clone();
        v2.data.start = "930".into();
        assert_eq!(range_of(&v2, &e), Some((e.start, e.end)));
        // Running entry: the start keeps its milliseconds.
        let r = entry(at(9, 9, 30) + 12_345, None);
        let v = edit_view(&r, &Catalog::default(), &prefs(), at(9, 12, 0));
        assert_eq!(range_of(&v, &r), Some((r.start, None)));
    }

    #[test]
    fn changed_start_only_keeps_end_instant() {
        let e = precise();
        let mut v = edit_view(&e, &Catalog::default(), &prefs(), at(9, 12, 0));
        v.data.start = "9:00".into();
        assert_eq!(range_of(&v, &e), Some((at(9, 9, 0), e.end)));
        // A new end is parsed, the unchanged start keeps its instant.
        let mut v = edit_view(&e, &Catalog::default(), &prefs(), at(9, 12, 0));
        v.data.end = "11:00".into();
        assert_eq!(range_of(&v, &e), Some((e.start, Some(at(9, 11, 0)))));
        // Another date: both are parsed.
        let mut v = edit_view(&e, &Catalog::default(), &prefs(), at(9, 12, 0));
        v.data.date = "2026-11-08".into();
        assert_eq!(range_of(&v, &e), Some((at(8, 9, 30), Some(at(8, 10, 15)))));
    }

    #[test]
    fn unchanged_time_in_repeated_dst_hour_is_kept() {
        // Europe/Berlin 2026-10-25: 02:00–03:00 happens twice; take the second 02:30.
        let tz = Tz::Europe__Berlin;
        let naive = date(2026, 10, 25).and_hms_opt(2, 30, 0).unwrap();
        let late = tz
            .from_local_datetime(&naive)
            .latest()
            .unwrap()
            .timestamp_millis();
        let e = entry(late + 1_000, Some(late + 20 * MINUTE_MS));
        let p = FormatPrefs { tz, ..prefs() };
        let v = edit_view(&e, &Catalog::default(), &p, late + HOUR_MS);
        let d = &v.data;
        assert_eq!(
            dialog_range_from(&d.date, &d.start, &d.end, tz, Some((e.start, e.end))),
            Some((e.start, e.end))
        );
    }

    #[test]
    fn editing_adjacent_timer_entries_does_not_overlap() {
        use tt_services::{EntryService, SystemClock};
        let svc = EntryService::new(SystemClock);
        let mut db = tt_storage::Db::open_in_memory().unwrap();
        // Switched tasks at the same millisecond (as `continue` / start do).
        let (t0, t1, t2) = (
            at(2, 9, 0) + 1_234,
            at(2, 9, 41) + 56_789,
            at(2, 10, 7) + 999,
        );
        let tpl = |d: &str| EntryTemplate {
            description: d.into(),
            ..Default::default()
        };
        svc.create_manual(&mut db, tpl("a"), t0, t1, MSK, true)
            .unwrap();
        let b = svc
            .create_manual(&mut db, tpl("b"), t1, t2, MSK, true)
            .unwrap()
            .remove(0);
        let mut v = edit_view(&b, &Catalog::default(), &prefs(), at(9, 12, 0));
        v.data.description = "b, renamed".into();
        let d = &v.data;
        // Re-parsing the minute alone would move the start into the previous entry.
        let (floored, _) = dialog_range(&d.date, &d.start, &d.end, MSK).unwrap();
        assert!(floored < t1);
        let (start, end) = range_of(&v, &b).unwrap();
        let edited = TimeEntry {
            description: d.description.to_string(),
            start,
            end,
            ..b.clone()
        };
        let saved = svc.update(&mut db, edited, MSK, true).unwrap();
        assert_eq!((saved[0].start, saved[0].end), (t1, Some(t2)));
        assert_eq!(saved[0].description, "b, renamed");
    }

    fn fake_days(dates: &[u32]) -> Vec<Day> {
        dates
            .iter()
            .map(|&d| (date(2026, 11, d), vec![], 0))
            .collect()
    }

    /// `fetch` over days 30, 29, … 1 of November with `batch` per page; counts calls.
    fn source(
        calls: &mut u32,
        batch: usize,
    ) -> impl FnMut(Option<NaiveDate>) -> Result<Vec<Day>, ()> + '_ {
        move |before| {
            *calls += 1;
            let all: Vec<u32> = (1..=30).rev().collect();
            let start = match before {
                None => 0,
                Some(b) => all
                    .iter()
                    .position(|&d| date(2026, 11, d) < b)
                    .unwrap_or(all.len()),
            };
            let end = (start + batch).min(all.len());
            Ok(fake_days(&all[start..end]))
        }
    }

    fn days_of(list: &[Day]) -> Vec<u32> {
        use chrono::Datelike;
        list.iter().map(|d| d.0.day()).collect()
    }

    #[test]
    fn page_bound_is_oldest_date_of_a_full_page() {
        assert_eq!(page_bound(&fake_days(&[9, 8]), 2), Some(date(2026, 11, 8)));
        assert_eq!(page_bound(&fake_days(&[9]), 2), None);
        assert_eq!(page_bound(&[], 2), None);
    }

    #[test]
    fn reload_fetches_back_to_the_bound() {
        let mut calls = 0;
        let (list, since) =
            reload_days(Some(date(2026, 11, 20)), 4, source(&mut calls, 4)).unwrap();
        assert_eq!(days_of(&list), (20..=30).rev().collect::<Vec<_>>());
        assert_eq!(since, Some(date(2026, 11, 20)));
        assert_eq!(calls, 3);
    }

    #[test]
    fn reload_without_bound_fetches_everything() {
        let mut calls = 0;
        let (list, since) = reload_days(None, 7, source(&mut calls, 7)).unwrap();
        assert_eq!(list.len(), 30);
        assert_eq!(since, None);
        assert_eq!(calls, 5);
    }

    #[test]
    fn reload_reaching_the_end_before_the_bound_is_complete() {
        // The bound is older than every stored day (e.g. its entries were deleted).
        let mut calls = 0;
        let (list, since) =
            reload_days(Some(date(2026, 10, 1)), 20, source(&mut calls, 20)).unwrap();
        assert_eq!(list.len(), 30);
        assert_eq!(since, None);
    }

    #[test]
    fn reload_keeps_the_bound_when_older_days_were_dropped() {
        let mut calls = 0;
        // One short page holds days older than the bound: they are not shown.
        let (list, since) =
            reload_days(Some(date(2026, 11, 25)), 40, source(&mut calls, 40)).unwrap();
        assert_eq!(days_of(&list), vec![30, 29, 28, 27, 26, 25]);
        assert_eq!(since, Some(date(2026, 11, 25)));
    }
}
