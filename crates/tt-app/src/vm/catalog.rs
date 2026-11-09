//! Rows and input parsing of the projects, clients and tags screens.

use tt_domain::{
    Client, FormatPrefs, Millis, NumberFormat, Project, Tag, Task, format_duration, format_money,
    format_number,
};

use crate::vm::entries::{hm_text, parse_duration_input};
use crate::vm::pickers::hex_color;
use crate::{ClientRowView, ProjectRowView, TagRowView, TaskRowView};

/// The typed text is not an amount / duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidInput;

/// Shown instead of a rate when the project has none.
const NO_RATE: &str = "—";

fn estimate_ms(estimate_s: Option<i64>) -> Option<i64> {
    estimate_s.filter(|s| *s > 0).map(|s| s * 1000)
}

/// Bar fill for `spent_ms` of the estimate: 0..=1 (capped), -1 without an estimate.
pub fn progress(spent_ms: Millis, estimate_s: Option<i64>) -> f32 {
    match estimate_ms(estimate_s) {
        Some(est) => (spent_ms.max(0) as f64 / est as f64).min(1.0) as f32,
        None => -1.0,
    }
}

/// The real percentage of the estimate (may exceed 100), rounded.
pub fn percent(spent_ms: Millis, estimate_s: Option<i64>) -> Option<u32> {
    estimate_ms(estimate_s).map(|est| (spent_ms.max(0) as f64 * 100.0 / est as f64).round() as u32)
}

/// More than 100 % of the estimate.
pub fn over_estimate(spent_ms: Millis, estimate_s: Option<i64>) -> bool {
    percent(spent_ms, estimate_s).is_some_and(|p| p > 100)
}

/// ("1:00", "150 %", bar fill, over) of an optional estimate; empty texts without one.
fn estimate_view(
    spent_ms: Millis,
    estimate_s: Option<i64>,
    prefs: &FormatPrefs,
) -> (String, String, f32, bool) {
    match percent(spent_ms, estimate_s) {
        Some(p) => (
            format_duration(estimate_s.unwrap_or_default(), prefs),
            format!("{p} %"),
            progress(spent_ms, estimate_s),
            over_estimate(spent_ms, estimate_s),
        ),
        None => (String::new(), String::new(), -1.0, false),
    }
}

/// Characters that only group digits and are ignored: spaces and apostrophes.
fn is_filler(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\'' | '\u{2019}')
}

/// "1,234" style grouping: 1–3 leading digits, then groups of exactly 3.
fn grouped_ok(s: &str, sep: char) -> bool {
    let mut parts = s.split(sep);
    let first = parts.next().unwrap_or_default();
    (1..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && parts.all(|p| p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Splits a cleaned amount into (integer digits, fraction digits) or `None`.
fn split_amount(s: &str, f: NumberFormat) -> Option<(String, String)> {
    let group_char = match f {
        NumberFormat::PointComma => Some('.'),
        NumberFormat::CommaPoint => Some(','),
        _ => None,
    };
    let (dots, commas) = (s.matches('.').count(), s.matches(',').count());
    let grouped = |sep: char| grouped_ok(s, sep).then(|| (s.replace(sep, ""), String::new()));
    let decimal_at = |pos: usize, group: Option<char>| {
        let int = &s[..pos];
        let int = match group {
            Some(g) if int.contains(g) => grouped_ok(int, g).then(|| int.replace(g, ""))?,
            _ => int.to_owned(),
        };
        Some((int, s[pos + 1..].to_owned()))
    };
    match (dots, commas) {
        (0, 0) => Some((s.to_owned(), String::new())),
        // Both kinds: the last one is the decimal separator, the other groups thousands.
        (_, c) if dots > 0 && c > 0 => {
            let (pos, group) = match (s.rfind('.'), s.rfind(',')) {
                (Some(d), Some(c)) if d > c => (d, ','),
                (_, Some(c)) => (c, '.'),
                _ => return None,
            };
            decimal_at(pos, Some(group))
        }
        (n, 0) | (0, n) => {
            let sep = if dots > 0 { '.' } else { ',' };
            let pos = s.find(sep)?;
            if n > 1 {
                return grouped(sep);
            }
            // A lone separator followed by exactly three digits is a thousands mark when
            // the number format groups with it.
            let thousands = group_char == Some(sep) && s.len() - pos - 1 == 3 && pos > 0;
            if thousands && let Some(g) = grouped(sep) {
                return Some(g);
            }
            decimal_at(pos, None)
        }
        _ => None,
    }
}

/// An amount typed by the user in cents: "50", "50.5", "50,50", "1 234,56", "1,234.56".
/// A lone "," or "." means decimals, except three digits after the number format's
/// thousands mark ("1,234" with `CommaPoint` is 1234). At most two decimals.
pub fn parse_money(s: &str, f: NumberFormat) -> Option<i64> {
    let s: String = s.chars().filter(|c| !is_filler(*c)).collect();
    let (int, frac) = split_amount(&s, f)?;
    let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
    if !digits(&int) || !digits(&frac) || frac.len() > 2 || int.len() > 12 {
        return None;
    }
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let whole: i64 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let cents: i64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<2}").parse().ok()?
    };
    Some(whole * 100 + cents)
}

/// An amount as shown in the input field (the user's separators).
pub fn money_text(cents: i64, f: NumberFormat) -> String {
    format_number(cents as f64 / 100.0, 2, f)
}

/// The hourly rate field: empty = no rate, else cents; `Err` for unparseable text.
pub fn parse_rate(s: &str, f: NumberFormat) -> Result<Option<i64>, InvalidInput> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    parse_money(s, f).map(Some).ok_or(InvalidInput)
}

/// The estimate field: empty or zero = no estimate, else seconds; `Err` if unparseable.
pub fn parse_estimate(s: &str) -> Result<Option<i64>, InvalidInput> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    parse_duration_input(s)
        .map(|secs| (secs > 0).then_some(secs))
        .ok_or(InvalidInput)
}

/// An estimate as shown in the input field: "H:MM".
pub fn estimate_text(seconds: i64) -> String {
    hm_text(seconds)
}

/// Case-insensitive search over the project name and its client's name; blank matches all.
pub fn matches_query(query: &str, name: &str, client: Option<&str>) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || name.to_lowercase().contains(&q)
        || client.is_some_and(|c| c.to_lowercase().contains(&q))
}

/// Projects matching `query` (and not archived unless `show_archived`), by name with the
/// archived ones last.
pub fn visible_projects<'a>(
    projects: &'a [Project],
    clients: &[Client],
    query: &str,
    show_archived: bool,
) -> Vec<&'a Project> {
    let client_name = |p: &Project| {
        p.client_id
            .and_then(|id| clients.iter().find(|c| c.id == id))
            .map(|c| c.name.as_str())
    };
    let mut out: Vec<&Project> = projects
        .iter()
        .filter(|p| show_archived || p.archived_at.is_none())
        .filter(|p| matches_query(query, &p.name, client_name(p)))
        .collect();
    out.sort_by_key(|p| (p.archived_at.is_some(), p.name.to_lowercase()));
    out
}

/// Clients (not archived unless `show_archived`) by name, archived last.
pub fn visible_clients(clients: &[Client], show_archived: bool) -> Vec<&Client> {
    let mut out: Vec<&Client> = clients
        .iter()
        .filter(|c| show_archived || c.archived_at.is_none())
        .collect();
    out.sort_by_key(|c| (c.archived_at.is_some(), c.name.to_lowercase()));
    out
}

/// Open tasks by name, then done ones by name.
pub fn sorted_tasks(tasks: &[Task]) -> Vec<&Task> {
    let mut out: Vec<&Task> = tasks.iter().collect();
    out.sort_by_key(|t| (t.done_at.is_some(), t.name.to_lowercase()));
    out
}

pub fn project_row(
    p: &Project,
    client: Option<&Client>,
    spent_ms: Millis,
    prefs: &FormatPrefs,
) -> ProjectRowView {
    let (estimate, percent, progress, over) = estimate_view(spent_ms, p.estimated_time, prefs);
    ProjectRowView {
        id: p.id.to_string().into(),
        name: p.name.as_str().into(),
        color: hex_color(&p.color),
        client: client.map_or("", |c| c.name.as_str()).into(),
        spent: format_duration(spent_ms / 1000, prefs).into(),
        estimate: estimate.into(),
        percent: percent.into(),
        progress,
        over,
        billable_rate: p
            .billable_rate
            .map_or_else(
                || NO_RATE.to_owned(),
                |r| format!("{}/{}", format_money(r, prefs), prefs.hour_suffix),
            )
            .into(),
        archived: p.archived_at.is_some(),
    }
}

pub fn task_row(t: &Task, spent_ms: Millis, prefs: &FormatPrefs) -> TaskRowView {
    let (estimate, percent, progress, over) = estimate_view(spent_ms, t.estimated_time, prefs);
    TaskRowView {
        id: t.id.to_string().into(),
        name: t.name.as_str().into(),
        spent: format_duration(spent_ms / 1000, prefs).into(),
        estimate: estimate.into(),
        percent: percent.into(),
        progress,
        over,
        done: t.done_at.is_some(),
    }
}

pub fn client_row(c: &Client, projects: usize) -> ClientRowView {
    ClientRowView {
        id: c.id.to_string().into(),
        name: c.name.as_str().into(),
        projects: projects as i32,
        archived: c.archived_at.is_some(),
    }
}

pub fn tag_row(t: &Tag, entries: i64) -> TagRowView {
    TagRowView {
        id: t.id.to_string().into(),
        name: t.name.as_str().into(),
        entries: entries as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::pickers::tests::{project, tag, task};
    use tt_domain::{Client, FormatPrefs, NumberFormat, new_id};

    fn client(name: &str) -> Client {
        Client {
            id: new_id(),
            name: name.into(),
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn progress_is_minus_one_without_estimate_and_capped_at_one() {
        assert_eq!(progress(10_000, None), -1.0);
        assert_eq!(progress(10_000, Some(0)), -1.0);
        assert_eq!(progress(0, Some(3600)), 0.0);
        assert_eq!(progress(1_800_000, Some(3600)), 0.5);
        assert_eq!(progress(3_600_000, Some(3600)), 1.0);
        // 150 %: the bar is full, the percentage is the real one.
        assert_eq!(progress(5_400_000, Some(3600)), 1.0);
        assert_eq!(percent(5_400_000, Some(3600)), Some(150));
        assert_eq!(percent(1_800_000, Some(3600)), Some(50));
        assert_eq!(percent(1, None), None);
    }

    #[test]
    fn over_estimate_means_more_than_one_hundred_percent() {
        assert!(over_estimate(5_400_000, Some(3600)));
        assert!(
            !over_estimate(3_600_000, Some(3600)),
            "exactly 100 % is fine"
        );
        assert!(!over_estimate(99_000_000, None));
    }

    #[test]
    fn money_accepts_the_usual_spellings() {
        let f = NumberFormat::CommaPoint;
        for (s, cents) in [
            ("50", 5000),
            ("50.5", 5050),
            ("50,50", 5050),
            ("1 234,56", 123_456),
            ("1,234.56", 123_456),
            ("1.234,56", 123_456),
            ("1,234", 123_400),
            ("1,234,567", 123_456_700),
            (" 7.05 ", 705),
            (".5", 50),
            ("0", 0),
            ("1\u{a0}234.5", 123_450),
        ] {
            assert_eq!(parse_money(s, f), Some(cents), "{s:?}");
        }
    }

    #[test]
    fn money_reads_a_lone_separator_by_the_number_format() {
        // "1.234": thousands with ".", a decimal point with "," or none.
        assert_eq!(
            parse_money("1.234", NumberFormat::PointComma),
            Some(123_400)
        );
        assert_eq!(parse_money("1.234", NumberFormat::CommaPoint), None);
        assert_eq!(parse_money("1,234", NumberFormat::SpaceComma), None);
        assert_eq!(parse_money("1,5", NumberFormat::SpaceComma), Some(150));
        assert_eq!(
            parse_money("1 234,5", NumberFormat::SpaceComma),
            Some(123_450)
        );
        assert_eq!(
            parse_money("1'234.5", NumberFormat::ApostrophePoint),
            Some(123_450)
        );
        assert_eq!(parse_money("50.5", NumberFormat::PointComma), Some(5050));
    }

    #[test]
    fn money_rejects_garbage() {
        for s in [
            "",
            " ",
            "abc",
            "-5",
            "+5",
            "5$",
            "1.2.3",
            "1,23,456",
            "50.555",
            ".",
            "1e3",
            "9999999999999",
        ] {
            assert_eq!(parse_money(s, NumberFormat::CommaPoint), None, "{s:?}");
        }
    }

    #[test]
    fn money_text_round_trips() {
        for f in [
            NumberFormat::PointComma,
            NumberFormat::CommaPoint,
            NumberFormat::SpaceComma,
            NumberFormat::SpacePoint,
            NumberFormat::ApostrophePoint,
        ] {
            for cents in [0, 5, 5000, 5050, 123_456, 100_000_000] {
                assert_eq!(parse_money(&money_text(cents, f), f), Some(cents), "{f:?}");
            }
        }
        assert_eq!(money_text(5000, NumberFormat::PointComma), "50,00");
    }

    #[test]
    fn rate_and_estimate_inputs_may_be_empty() {
        let f = NumberFormat::CommaPoint;
        assert_eq!(parse_rate(" ", f), Ok(None));
        assert_eq!(parse_rate("12.5", f), Ok(Some(1250)));
        assert_eq!(parse_rate("x", f), Err(InvalidInput));
        assert_eq!(parse_estimate(""), Ok(None));
        assert_eq!(parse_estimate("0"), Ok(None));
        assert_eq!(parse_estimate("40"), Ok(Some(144_000)));
        assert_eq!(parse_estimate("1:30"), Ok(Some(5400)));
        assert_eq!(parse_estimate("soon"), Err(InvalidInput));
        assert_eq!(estimate_text(144_000), "40:00");
    }

    #[test]
    fn search_ignores_case_and_looks_at_name_and_client() {
        assert!(matches_query("", "Website", None));
        assert!(matches_query("  ", "Website", None));
        assert!(matches_query("WEB", "website redesign", None));
        assert!(matches_query("acme", "Website", Some("ACME Corp")));
        assert!(matches_query("я", "Сайт Я", None));
        assert!(!matches_query("zzz", "Website", Some("Acme")));
        assert!(!matches_query("acme", "Website", None));
    }

    #[test]
    fn visible_projects_filter_sort_and_put_archived_last() {
        let acme = client("Acme");
        let mut web = project("Website", "#111111");
        web.client_id = Some(acme.id);
        let app = project("app", "#222222");
        let mut old = project("Archived one", "#333333");
        old.archived_at = Some(5);
        let all = [web.clone(), old.clone(), app.clone()];
        let clients = [acme];
        let names = |q: &str, archived: bool| -> Vec<String> {
            visible_projects(&all, &clients, q, archived)
                .iter()
                .map(|p| p.name.clone())
                .collect()
        };
        assert_eq!(names("", false), ["app", "Website"]);
        assert_eq!(names("", true), ["app", "Website", "Archived one"]);
        assert_eq!(names("ACME", false), ["Website"]);
        assert_eq!(names("one", false), Vec::<String>::new());
        assert_eq!(names("one", true), ["Archived one"]);
    }

    #[test]
    fn project_row_formats_spent_estimate_and_rate() {
        let prefs = FormatPrefs::default();
        let mut p = project("Site", "#8b5cf6");
        p.estimated_time = Some(3600);
        p.billable_rate = Some(5050);
        let c = client("Acme");
        let row = project_row(&p, Some(&c), 5_400_000, &prefs);
        assert_eq!(row.name, "Site");
        assert_eq!(row.client, "Acme");
        assert_eq!(row.spent, "1:30");
        assert_eq!(row.estimate, "1:00");
        assert_eq!(row.percent, "150 %");
        assert_eq!(row.progress, 1.0);
        assert!(row.over && !row.archived);
        assert_eq!(row.billable_rate, "$50.50/h");

        p.estimated_time = None;
        p.billable_rate = None;
        p.archived_at = Some(1);
        let row = project_row(&p, None, 0, &prefs);
        assert_eq!(row.client, "");
        assert_eq!((row.estimate.as_str(), row.percent.as_str()), ("", ""));
        assert_eq!(row.progress, -1.0);
        assert!(!row.over && row.archived);
        assert_eq!(row.billable_rate, "—");
    }

    #[test]
    fn tasks_list_open_first_then_done_by_name() {
        let p = project("p", "#000000");
        let b = task(&p, "b");
        let a = task(&p, "A");
        let mut done = task(&p, "0 done");
        done.done_at = Some(1);
        let all = [done.clone(), b, a];
        let sorted = sorted_tasks(&all);
        let names: Vec<_> = sorted.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["A", "b", "0 done"]);

        let row = task_row(&done, 0, &FormatPrefs::default());
        assert!(row.done);
        assert_eq!(row.progress, -1.0);
    }

    #[test]
    fn client_and_tag_rows() {
        let mut c = client("Acme");
        c.archived_at = Some(1);
        let row = client_row(&c, 3);
        assert_eq!(
            (row.name.as_str(), row.projects, row.archived),
            ("Acme", 3, true)
        );
        let row = tag_row(&tag("urgent"), 7);
        assert_eq!((row.name.as_str(), row.entries), ("urgent", 7));

        let (a, b) = (client("b"), client("A"));
        let mut z = client("z");
        z.archived_at = Some(1);
        let all = [z, a, b];
        let names = |archived| -> Vec<String> {
            visible_clients(&all, archived)
                .iter()
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(false), ["A", "b"]);
        assert_eq!(names(true), ["A", "b", "z"]);
    }
}
