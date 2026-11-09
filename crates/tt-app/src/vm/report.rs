//! Reports screen: period presets, the saved filter state, group names and the view of a
//! report result (bars, pie, flattened table).

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use slint::{Color, ModelRc, VecModel};
use tt_domain::{
    Client, DateFormat, FormatPrefs, GroupBy, GroupKey, Id, Interval, ReportFilter, ReportResult,
    RoundingMode, TagMatch, Tz, format_date, format_duration, start_of_local_day,
};

use crate::vm::charts::{CAPTION_SEPARATOR, bars_normalized, money, pie_slices};
use crate::vm::dates::{iso, parse_iso};
use crate::vm::pickers::{Catalog, hex_color, option_label, palette_color};
use crate::{BarView, FilterView, PieSlice, ReportRowView, TagOption};

/// Setting that holds the last report state as JSON.
pub const SETTING_KEY: &str = "report.last";

/// Rounding steps offered, minutes.
pub const STEPS: [u32; 7] = [1, 5, 6, 10, 15, 30, 60];
const DEFAULT_STEP: u32 = 15;
/// Groups the pie shows; the rest is summed into "Other".
const PIE_SLICES: usize = 8;
/// Index of "Custom range" in the period select.
pub const CUSTOM_PRESET: i32 = 7;

/// Group choices in the order of the UI values (0 …).
pub const GROUPS: [GroupBy; 7] = [
    GroupBy::Project,
    GroupBy::Task,
    GroupBy::Client,
    GroupBy::Tag,
    GroupBy::Billable,
    GroupBy::Description,
    GroupBy::Day,
];
const INTERVALS: [Interval; 3] = [Interval::Day, Interval::Week, Interval::Month];
const MODES: [RoundingMode; 3] = [RoundingMode::Up, RoundingMode::Down, RoundingMode::Nearest];

/// Period presets in the order of the UI values (0 …; 7 is the custom range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
}

pub const PRESETS: [Preset; 7] = [
    Preset::Today,
    Preset::Yesterday,
    Preset::ThisWeek,
    Preset::LastWeek,
    Preset::ThisMonth,
    Preset::LastMonth,
    Preset::ThisYear,
];

fn last_day_of_month(first: NaiveDate) -> NaiveDate {
    (first + Months::new(1)) - Days::new(1)
}

/// First and last day (inclusive) of the preset around `today`; weeks start on `week_start`.
pub fn preset_range(p: Preset, today: NaiveDate, week_start: Weekday) -> (NaiveDate, NaiveDate) {
    let week = |d: NaiveDate| {
        let back = d.weekday().days_since(week_start);
        let first = d - Days::new(u64::from(back));
        (first, first + Days::new(6))
    };
    let month = |d: NaiveDate| {
        let first = d.with_day(1).expect("day 1 exists");
        (first, last_day_of_month(first))
    };
    match p {
        Preset::Today => (today, today),
        Preset::Yesterday => {
            let d = today - Days::new(1);
            (d, d)
        }
        Preset::ThisWeek => week(today),
        Preset::LastWeek => week(today - Days::new(7)),
        Preset::ThisMonth => month(today),
        Preset::LastMonth => month(today - Months::new(1)),
        Preset::ThisYear => (
            NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("Jan 1 exists"),
            NaiveDate::from_ymd_opt(today.year(), 12, 31).expect("Dec 31 exists"),
        ),
    }
}

/// The report screen's state, saved under [`SETTING_KEY`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportState {
    /// `None` = the custom range.
    pub preset: Option<Preset>,
    /// ISO dates of the custom range.
    pub custom_from: String,
    pub custom_to: String,
    pub project_ids: Vec<Id>,
    pub task_ids: Vec<Id>,
    pub client_ids: Vec<Id>,
    pub tag_ids: Vec<Id>,
    pub tag_match: TagMatch,
    pub billable: Option<bool>,
    pub g1: GroupBy,
    pub g2: Option<GroupBy>,
    pub interval: Interval,
    pub rounding: Option<(RoundingMode, u32)>,
}

impl Default for ReportState {
    fn default() -> Self {
        Self {
            preset: Some(Preset::ThisWeek),
            custom_from: String::new(),
            custom_to: String::new(),
            project_ids: Vec::new(),
            task_ids: Vec::new(),
            client_ids: Vec::new(),
            tag_ids: Vec::new(),
            tag_match: TagMatch::Any,
            billable: None,
            g1: GroupBy::Project,
            g2: Some(GroupBy::Task),
            interval: Interval::Day,
            rounding: None,
        }
    }
}

impl ReportState {
    /// The saved state; missing, garbage or inconsistent values fall back to the defaults.
    pub fn parse(json: Option<&str>) -> Self {
        json.and_then(|j| serde_json::from_str::<Self>(j).ok())
            .unwrap_or_default()
            .normalized()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("report state serializes")
    }

    /// Makes the state consistent: group 2 differs from group 1 and the rounding step is
    /// one of [`STEPS`].
    pub fn normalized(mut self) -> Self {
        if self.g2 == Some(self.g1) {
            self.g2 = None;
        }
        if let Some((_, step)) = &mut self.rounding
            && !STEPS.contains(step)
        {
            *step = DEFAULT_STEP;
        }
        self
    }

    /// First and last day of the period (inclusive). An unset or invalid custom date is
    /// `today` (the end: the start); an end before the start is the start.
    pub fn range(&self, today: NaiveDate, week_start: Weekday) -> (NaiveDate, NaiveDate) {
        match self.preset {
            Some(p) => preset_range(p, today, week_start),
            None => {
                let from = parse_iso(&self.custom_from).unwrap_or(today);
                let to = parse_iso(&self.custom_to).unwrap_or(from);
                (from, to.max(from))
            }
        }
    }

    /// The report filter: the period from the start of the first day to the start of the day
    /// after the last one, in `tz`.
    pub fn filter(&self, today: NaiveDate, week_start: Weekday, tz: Tz) -> ReportFilter {
        let (from, to) = self.range(today, week_start);
        ReportFilter {
            start: start_of_local_day(from, tz),
            end: start_of_local_day(to + Days::new(1), tz),
            project_ids: self.project_ids.clone(),
            task_ids: self.task_ids.clone(),
            client_ids: self.client_ids.clone(),
            tag_ids: self.tag_ids.clone(),
            tag_match: self.tag_match,
            billable: self.billable,
            rounding: self.rounding,
        }
    }

    /// Keeps only the filter ids that exist (an item deleted since the state was saved
    /// would otherwise silently empty the report).
    pub fn retain_existing(&mut self, c: &Catalog, clients: &[Client]) {
        self.project_ids
            .retain(|id| c.projects.iter().any(|p| p.id == *id));
        self.task_ids
            .retain(|id| c.tasks.iter().any(|t| t.id == *id));
        self.client_ids
            .retain(|id| clients.iter().any(|x| x.id == *id));
        self.tag_ids.retain(|id| c.tags.iter().any(|t| t.id == *id));
    }

    // UI value conversions -------------------------------------------------------------

    /// UI value of the period select: 0 … 6 presets, 7 custom.
    pub fn preset_value(&self) -> i32 {
        self.preset
            .and_then(|p| PRESETS.iter().position(|x| *x == p))
            .map_or(CUSTOM_PRESET, |i| i as i32)
    }

    pub fn set_preset_value(&mut self, v: i32) {
        self.preset = usize::try_from(v)
            .ok()
            .and_then(|i| PRESETS.get(i))
            .copied();
    }

    pub fn billable_value(&self) -> i32 {
        match self.billable {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        }
    }

    pub fn set_billable_value(&mut self, v: i32) {
        self.billable = match v {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        };
    }

    pub fn set_groups(&mut self, g1: i32, g2: i32) {
        self.g1 = group_from_value(g1).unwrap_or(GroupBy::Project);
        self.g2 = group_from_value(g2);
    }

    pub fn interval_value(&self) -> i32 {
        position(&INTERVALS, self.interval)
    }

    pub fn set_interval_value(&mut self, v: i32) {
        self.interval = usize::try_from(v)
            .ok()
            .and_then(|i| INTERVALS.get(i))
            .copied()
            .unwrap_or(Interval::Day);
    }

    /// -1 = none, else the index of the mode (up, down, nearest).
    pub fn rounding_mode_value(&self) -> i32 {
        self.rounding.map_or(-1, |(m, _)| position(&MODES, m))
    }

    /// Minutes; [`DEFAULT_STEP`] while rounding is off.
    pub fn rounding_step_value(&self) -> i32 {
        self.rounding.map_or(DEFAULT_STEP, |(_, s)| s) as i32
    }

    pub fn set_rounding_values(&mut self, mode: i32, step: i32) {
        self.rounding = usize::try_from(mode)
            .ok()
            .and_then(|i| MODES.get(i))
            .map(|m| (*m, u32::try_from(step).unwrap_or(DEFAULT_STEP)));
    }
}

fn position<T: PartialEq>(all: &[T], v: T) -> i32 {
    all.iter().position(|x| *x == v).unwrap_or(0) as i32
}

/// UI value (0 …) of a grouping.
pub fn group_value(g: GroupBy) -> i32 {
    position(&GROUPS, g)
}

/// The grouping of a UI value; `None` for -1 and anything out of range.
pub fn group_from_value(v: i32) -> Option<GroupBy> {
    usize::try_from(v).ok().and_then(|i| GROUPS.get(i)).copied()
}

// Names -------------------------------------------------------------------------------

/// Texts for empty groups and the report's fixed labels.
#[derive(Debug, Clone, Default)]
pub struct Labels {
    /// January … December, short.
    pub months: [String; 12],
    pub total: String,
    pub other: String,
    pub no_project: String,
    pub no_task: String,
    pub no_client: String,
    pub no_tag: String,
    pub no_description: String,
    pub billable: String,
    pub non_billable: String,
}

/// Display names and colors of group keys.
pub struct Names<'a> {
    pub catalog: &'a Catalog,
    pub clients: &'a [Client],
    pub labels: &'a Labels,
    pub date: DateFormat,
}

impl Names<'_> {
    /// Text of the `key` of a group-by `by`: project, task ("Project › Task"), client and
    /// tag names, a date in the user's format, "Billable" / "Non-billable", the
    /// description, or the "No …" label of the missing value.
    pub fn name(&self, by: GroupBy, key: &GroupKey) -> String {
        let l = self.labels;
        match key {
            GroupKey::None => match by {
                GroupBy::Project => &l.no_project,
                GroupBy::Task => &l.no_task,
                GroupBy::Client => &l.no_client,
                GroupBy::Tag => &l.no_tag,
                GroupBy::Description => &l.no_description,
                GroupBy::Billable | GroupBy::Day => return String::new(),
            }
            .clone(),
            GroupKey::Id(id) => self.id_name(by, *id),
            GroupKey::Bool(true) => l.billable.clone(),
            GroupKey::Bool(false) => l.non_billable.clone(),
            GroupKey::Text(t) => t.clone(),
            GroupKey::Date(d) => format_date(*d, self.date),
        }
    }

    fn id_name(&self, by: GroupBy, id: Id) -> String {
        let c = self.catalog;
        let name = match by {
            GroupBy::Project => c.project(id).map(|p| p.name.clone()),
            GroupBy::Task => c.task(id).map(|t| match c.project(t.project_id) {
                Some(p) => option_label(p, Some(t)),
                None => t.name.clone(),
            }),
            GroupBy::Client => self
                .clients
                .iter()
                .find(|x| x.id == id)
                .map(|x| x.name.clone()),
            GroupBy::Tag => c.tags.iter().find(|t| t.id == id).map(|t| t.name.clone()),
            _ => None,
        };
        name.unwrap_or_default()
    }

    /// Project color of a project or task group; transparent for anything else.
    pub fn color(&self, by: GroupBy, key: &GroupKey) -> Color {
        let GroupKey::Id(id) = key else {
            return Color::default();
        };
        let project = match by {
            GroupBy::Project => self.catalog.project(*id),
            GroupBy::Task => self
                .catalog
                .task(*id)
                .and_then(|t| self.catalog.project(t.project_id)),
            _ => None,
        };
        project.map_or_else(Color::default, |p| hex_color(&p.color))
    }
}

// Filter options ----------------------------------------------------------------------

fn filter_view(mut items: Vec<(Id, String)>, selected: &[Id]) -> FilterView {
    items.sort_by_key(|(_, name)| name.to_lowercase());
    let text = items
        .iter()
        .filter(|(id, _)| selected.contains(id))
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let options: Vec<TagOption> = items
        .iter()
        .map(|(id, name)| TagOption {
            id: id.to_string().into(),
            name: name.as_str().into(),
            selected: selected.contains(id),
        })
        .collect();
    FilterView {
        options: ModelRc::new(VecModel::from(options)),
        text: text.into(),
    }
}

/// All projects (archived too: they have history) by name.
pub fn project_filter(c: &Catalog, selected: &[Id]) -> FilterView {
    filter_view(
        c.projects.iter().map(|p| (p.id, p.name.clone())).collect(),
        selected,
    )
}

/// All tasks as "Project › Task" by label.
pub fn task_filter(c: &Catalog, selected: &[Id]) -> FilterView {
    let items = c
        .tasks
        .iter()
        .map(|t| {
            let label = match c.project(t.project_id) {
                Some(p) => option_label(p, Some(t)),
                None => t.name.clone(),
            };
            (t.id, label)
        })
        .collect();
    filter_view(items, selected)
}

pub fn client_filter(clients: &[Client], selected: &[Id]) -> FilterView {
    filter_view(
        clients.iter().map(|c| (c.id, c.name.clone())).collect(),
        selected,
    )
}

pub fn tag_filter(c: &Catalog, selected: &[Id]) -> FilterView {
    filter_view(
        c.tags.iter().map(|t| (t.id, t.name.clone())).collect(),
        selected,
    )
}

// Result view -------------------------------------------------------------------------

/// Everything the report shows.
#[derive(Debug, Clone, Default)]
pub struct ReportView {
    pub total: String,
    /// Empty when nothing is billable.
    pub cost: String,
    pub bars: Vec<BarView>,
    pub pie: Vec<PieSlice>,
    pub rows: Vec<ReportRowView>,
}

/// The tree of groups as a list: each group followed by its sub-groups (`level` 1), the
/// total last (level 0, transparent color).
pub fn flatten_rows(
    r: &ReportResult,
    g1: GroupBy,
    g2: Option<GroupBy>,
    names: &Names,
    prefs: &FormatPrefs,
) -> Vec<ReportRowView> {
    let row = |level: i32, label: String, color: Color, seconds: i64, cost: i64| ReportRowView {
        level,
        label: label.into(),
        color,
        duration: format_duration(seconds, prefs).into(),
        cost: money(cost, prefs).into(),
    };
    let sub = g2.unwrap_or(g1);
    let mut rows = Vec::new();
    for g in &r.groups {
        rows.push(row(
            0,
            names.name(g1, &g.key),
            names.color(g1, &g.key),
            g.seconds,
            g.cost_cents,
        ));
        for c in &g.children {
            rows.push(row(
                1,
                names.name(sub, &c.key),
                names.color(sub, &c.key),
                c.seconds,
                c.cost_cents,
            ));
        }
    }
    rows.push(row(
        0,
        names.labels.total.clone(),
        Color::default(),
        r.total_seconds,
        r.total_cost_cents,
    ));
    rows
}

/// Label under a bar: "dd.mm" of the day or of the week's first day, the short month name
/// for a month.
fn bucket_label(d: NaiveDate, interval: Interval, l: &Labels) -> String {
    match interval {
        Interval::Day | Interval::Week => format!("{:02}.{:02}", d.day(), d.month()),
        Interval::Month => l.months[d.month0() as usize].clone(),
    }
}

/// Hover text of a bar's period: the date, the week's first and last day, or month and year.
fn bucket_caption(d: NaiveDate, interval: Interval, l: &Labels, date: DateFormat) -> String {
    match interval {
        Interval::Day => format_date(d, date),
        Interval::Week => format!(
            "{} – {}",
            format_date(d, date),
            format_date(d + Days::new(6), date)
        ),
        Interval::Month => format!("{} {}", l.months[d.month0() as usize], d.year()),
    }
}

fn bars(r: &ReportResult, interval: Interval, l: &Labels, prefs: &FormatPrefs) -> Vec<BarView> {
    let seconds: Vec<i64> = r.buckets.iter().map(|b| b.seconds).collect();
    r.buckets
        .iter()
        .zip(bars_normalized(&seconds))
        .map(|(b, value)| BarView {
            label: bucket_label(b.start, interval, l).into(),
            value,
            caption: format!(
                "{}{CAPTION_SEPARATOR}{}",
                bucket_caption(b.start, interval, l, prefs.date),
                format_duration(b.seconds, prefs)
            )
            .into(),
        })
        .collect()
}

/// Pie of the first grouping: project colors where there are some, else the palette in
/// turn; groups past [`PIE_SLICES`] are summed into "Other".
fn pie(r: &ReportResult, g1: GroupBy, names: &Names, prefs: &FormatPrefs) -> Vec<PieSlice> {
    let groups: Vec<_> = r.groups.iter().filter(|g| g.seconds > 0).collect();
    let sum: i64 = groups.iter().map(|g| g.seconds).sum();
    let mut values: Vec<(String, i64, Color)> = groups
        .iter()
        .take(PIE_SLICES)
        .enumerate()
        .map(|(i, g)| {
            let color = match (names.color(g1, &g.key), &g.key) {
                (c, _) if c.alpha() > 0 => c,
                (_, GroupKey::None) => Color::default(),
                _ => hex_color(palette_color(i)),
            };
            (names.name(g1, &g.key), g.seconds, color)
        })
        .collect();
    let rest: i64 = groups.iter().skip(PIE_SLICES).map(|g| g.seconds).sum();
    if rest > 0 {
        values.push((names.labels.other.clone(), rest, Color::default()));
    }
    let mut slices = pie_slices(&values);
    for (slice, (label, seconds, _)) in slices.iter_mut().zip(&values) {
        let percent = (seconds * 100 + sum.max(1) / 2) / sum.max(1);
        slice.caption = format!(
            "{label}{CAPTION_SEPARATOR}{}{CAPTION_SEPARATOR}{percent} %",
            format_duration(*seconds, prefs)
        )
        .into();
    }
    slices
}

/// Maps a report result to what the screen shows.
pub fn report_view(
    r: &ReportResult,
    g1: GroupBy,
    g2: Option<GroupBy>,
    interval: Interval,
    names: &Names,
    prefs: &FormatPrefs,
) -> ReportView {
    ReportView {
        total: format_duration(r.total_seconds, prefs),
        cost: money(r.total_cost_cents, prefs),
        bars: bars(r, interval, names.labels, prefs),
        pie: pie(r, g1, names, prefs),
        rows: flatten_rows(r, g1, g2, names, prefs),
    }
}

/// The period in the user's date format: one date, or "from – to".
pub fn range_text(from: NaiveDate, to: NaiveDate, date: DateFormat) -> String {
    if from == to {
        format_date(from, date)
    } else {
        format!("{} – {}", format_date(from, date), format_date(to, date))
    }
}

/// `to_iso` for the bridge's custom date fields.
pub fn custom_dates(from: NaiveDate, to: NaiveDate) -> (String, String) {
    (iso(from).to_string(), iso(to).to_string())
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)] // tests tweak a few fields of the default state
mod tests {
    use super::*;
    use crate::vm::pickers::tests::{project, task};
    use slint::Model;
    use tt_domain::{GroupRow, ReportResult, new_id};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn range(p: Preset, today: NaiveDate, ws: Weekday) -> (NaiveDate, NaiveDate) {
        preset_range(p, today, ws)
    }

    #[test]
    fn day_presets() {
        let today = d(2026, 11, 9);
        assert_eq!(range(Preset::Today, today, Weekday::Mon), (today, today));
        let y = d(2026, 11, 8);
        assert_eq!(range(Preset::Yesterday, today, Weekday::Mon), (y, y));
        // Across a year boundary.
        let jan1 = d(2026, 1, 1);
        let y = d(2025, 12, 31);
        assert_eq!(range(Preset::Yesterday, jan1, Weekday::Mon), (y, y));
    }

    #[test]
    fn week_presets_follow_the_week_start() {
        // 2026-11-09 is a Monday, 2026-11-15 a Sunday.
        let monday = d(2026, 11, 9);
        let sunday = d(2026, 11, 15);
        let wed = d(2026, 11, 11);
        for today in [monday, wed, sunday] {
            assert_eq!(
                range(Preset::ThisWeek, today, Weekday::Mon),
                (monday, sunday)
            );
            assert_eq!(
                range(Preset::LastWeek, today, Weekday::Mon),
                (d(2026, 11, 2), d(2026, 11, 8))
            );
        }
        // Weeks starting on Sunday: the Sunday 15th opens the next week.
        assert_eq!(
            range(Preset::ThisWeek, wed, Weekday::Sun),
            (d(2026, 11, 8), d(2026, 11, 14))
        );
        assert_eq!(
            range(Preset::ThisWeek, sunday, Weekday::Sun),
            (sunday, d(2026, 11, 21))
        );
        assert_eq!(
            range(Preset::LastWeek, sunday, Weekday::Sun),
            (d(2026, 11, 8), d(2026, 11, 14))
        );
        assert_eq!(
            range(Preset::LastWeek, monday, Weekday::Sun),
            (d(2026, 11, 1), d(2026, 11, 7))
        );
    }

    #[test]
    fn week_presets_across_the_year_boundary() {
        // Jan 1, 2026 is a Thursday.
        let jan1 = d(2026, 1, 1);
        assert_eq!(
            range(Preset::ThisWeek, jan1, Weekday::Mon),
            (d(2025, 12, 29), d(2026, 1, 4))
        );
        assert_eq!(
            range(Preset::LastWeek, jan1, Weekday::Mon),
            (d(2025, 12, 22), d(2025, 12, 28))
        );
        assert_eq!(
            range(Preset::ThisWeek, jan1, Weekday::Sun),
            (d(2025, 12, 28), d(2026, 1, 3))
        );
        // Dec 31, 2026 is a Thursday.
        let dec31 = d(2026, 12, 31);
        assert_eq!(
            range(Preset::ThisWeek, dec31, Weekday::Mon),
            (d(2026, 12, 28), d(2027, 1, 3))
        );
        assert_eq!(
            range(Preset::ThisWeek, dec31, Weekday::Sun),
            (d(2026, 12, 27), d(2027, 1, 2))
        );
        assert_eq!(
            range(Preset::LastWeek, dec31, Weekday::Sun),
            (d(2026, 12, 20), d(2026, 12, 26))
        );
    }

    #[test]
    fn month_and_year_presets() {
        let today = d(2026, 11, 9);
        assert_eq!(
            range(Preset::ThisMonth, today, Weekday::Mon),
            (d(2026, 11, 1), d(2026, 11, 30))
        );
        assert_eq!(
            range(Preset::LastMonth, today, Weekday::Mon),
            (d(2026, 10, 1), d(2026, 10, 31))
        );
        assert_eq!(
            range(Preset::ThisYear, today, Weekday::Sun),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
        // January: last month is December of the previous year.
        let jan1 = d(2026, 1, 1);
        assert_eq!(
            range(Preset::ThisMonth, jan1, Weekday::Mon),
            (jan1, d(2026, 1, 31))
        );
        assert_eq!(
            range(Preset::LastMonth, jan1, Weekday::Mon),
            (d(2025, 12, 1), d(2025, 12, 31))
        );
        // December 31 and a leap February.
        let dec31 = d(2026, 12, 31);
        assert_eq!(
            range(Preset::ThisMonth, dec31, Weekday::Mon),
            (d(2026, 12, 1), dec31)
        );
        assert_eq!(
            range(Preset::LastMonth, dec31, Weekday::Mon),
            (d(2026, 11, 1), d(2026, 11, 30))
        );
        assert_eq!(
            range(Preset::ThisYear, dec31, Weekday::Mon),
            (d(2026, 1, 1), dec31)
        );
        assert_eq!(
            range(Preset::LastMonth, d(2028, 3, 31), Weekday::Mon),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
    }

    #[test]
    fn filter_covers_whole_local_days_half_open() {
        let tz = Tz::Europe__Moscow;
        let mut s = ReportState::default();
        s.preset = None;
        s.custom_from = "2026-11-09".into();
        s.custom_to = "2026-11-10".into();
        let f = s.filter(d(2026, 11, 20), Weekday::Mon, tz);
        assert_eq!(f.start, start_of_local_day(d(2026, 11, 9), tz));
        assert_eq!(f.end, start_of_local_day(d(2026, 11, 11), tz));
        // An end before the start is the start; garbage dates are today.
        s.custom_to = "2026-11-01".into();
        assert_eq!(
            s.range(d(2026, 11, 20), Weekday::Mon),
            (d(2026, 11, 9), d(2026, 11, 9))
        );
        s.custom_from = "junk".into();
        s.custom_to = String::new();
        assert_eq!(
            s.range(d(2026, 11, 20), Weekday::Mon),
            (d(2026, 11, 20), d(2026, 11, 20))
        );
    }

    #[test]
    fn state_round_trips_and_tolerates_garbage() {
        let mut s = ReportState::default();
        s.preset = None;
        s.custom_from = "2026-01-01".into();
        s.project_ids = vec![new_id()];
        s.tag_ids = vec![new_id(), new_id()];
        s.tag_match = TagMatch::All;
        s.billable = Some(false);
        s.g1 = GroupBy::Client;
        s.g2 = Some(GroupBy::Day);
        s.interval = Interval::Month;
        s.rounding = Some((RoundingMode::Nearest, 30));
        assert_eq!(ReportState::parse(Some(&s.to_json())), s);

        let defaults = ReportState::default();
        assert_eq!(defaults.preset, Some(Preset::ThisWeek));
        assert_eq!(
            (defaults.g1, defaults.g2),
            (GroupBy::Project, Some(GroupBy::Task))
        );
        assert_eq!(defaults.interval, Interval::Day);
        assert_eq!(defaults.rounding, None);
        for junk in [
            "",
            "not json",
            "[1,2]",
            "{\"g1\": \"nonsense\"}",
            "{\"preset\": 5}",
        ] {
            assert_eq!(ReportState::parse(Some(junk)), defaults, "{junk}");
        }
        assert_eq!(ReportState::parse(None), defaults);
        // Missing keys keep their defaults.
        let partial = ReportState::parse(Some("{\"interval\": \"week\"}"));
        assert_eq!(partial.interval, Interval::Week);
        assert_eq!(partial.preset, Some(Preset::ThisWeek));
    }

    #[test]
    fn normalizing_drops_a_group_equal_to_the_first_and_bad_steps() {
        let mut s = ReportState::default();
        s.g1 = GroupBy::Task;
        s.g2 = Some(GroupBy::Task);
        s.rounding = Some((RoundingMode::Up, 7));
        let n = s.normalized();
        assert_eq!(n.g2, None);
        assert_eq!(n.rounding, Some((RoundingMode::Up, 15)));
        let saved = ReportState::parse(Some(
            "{\"g1\":\"tag\",\"g2\":\"tag\",\"rounding\":[\"down\",0]}",
        ));
        assert_eq!(saved.g2, None);
        assert_eq!(saved.rounding, Some((RoundingMode::Down, 15)));
    }

    #[test]
    fn ui_values_convert_both_ways() {
        let mut s = ReportState::default();
        assert_eq!(s.preset_value(), 2);
        s.set_preset_value(7);
        assert_eq!((s.preset, s.preset_value()), (None, 7));
        s.set_preset_value(0);
        assert_eq!(s.preset, Some(Preset::Today));
        s.set_groups(6, -1);
        assert_eq!((s.g1, s.g2), (GroupBy::Day, None));
        assert_eq!(group_value(GroupBy::Tag), 3);
        s.set_billable_value(2);
        assert_eq!((s.billable, s.billable_value()), (Some(false), 2));
        s.set_rounding_values(2, 10);
        assert_eq!(s.rounding, Some((RoundingMode::Nearest, 10)));
        assert_eq!((s.rounding_mode_value(), s.rounding_step_value()), (2, 10));
        s.set_rounding_values(-1, 10);
        assert_eq!((s.rounding, s.rounding_mode_value()), (None, -1));
        s.set_interval_value(2);
        assert_eq!((s.interval, s.interval_value()), (Interval::Month, 2));
    }

    struct Fx {
        catalog: Catalog,
        clients: Vec<Client>,
        labels: Labels,
    }

    fn fx() -> Fx {
        let web = project("Website", "#ff0000");
        let api = project("API", "#00ff00");
        let login = task(&web, "Login");
        let client = Client {
            id: new_id(),
            name: "Acme".into(),
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let labels = Labels {
            total: "Total".into(),
            other: "Other".into(),
            no_project: "No project".into(),
            no_task: "No task".into(),
            no_client: "No client".into(),
            no_tag: "No tag".into(),
            no_description: "No description".into(),
            billable: "Billable".into(),
            non_billable: "Non-billable".into(),
            months: std::array::from_fn(|i| format!("M{}", i + 1)),
        };
        Fx {
            catalog: Catalog {
                projects: vec![web, api],
                tasks: vec![login],
                tags: vec![],
            },
            clients: vec![client],
            labels,
        }
    }

    impl Fx {
        fn names(&self) -> Names<'_> {
            Names {
                catalog: &self.catalog,
                clients: &self.clients,
                labels: &self.labels,
                date: DateFormat::PointDMY,
            }
        }
    }

    fn row(key: GroupKey, seconds: i64, cost: i64, children: Vec<GroupRow>) -> GroupRow {
        GroupRow {
            key,
            seconds,
            cost_cents: cost,
            children,
        }
    }

    #[test]
    fn names_map_group_keys() {
        let f = fx();
        let n = f.names();
        let web = f.catalog.projects[0].id;
        let login = f.catalog.tasks[0].id;
        assert_eq!(n.name(GroupBy::Project, &GroupKey::Id(web)), "Website");
        assert_eq!(
            n.name(GroupBy::Task, &GroupKey::Id(login)),
            "Website › Login"
        );
        assert_eq!(
            n.name(GroupBy::Client, &GroupKey::Id(f.clients[0].id)),
            "Acme"
        );
        assert_eq!(n.name(GroupBy::Project, &GroupKey::None), "No project");
        assert_eq!(n.name(GroupBy::Task, &GroupKey::None), "No task");
        assert_eq!(n.name(GroupBy::Client, &GroupKey::None), "No client");
        assert_eq!(n.name(GroupBy::Tag, &GroupKey::None), "No tag");
        assert_eq!(
            n.name(GroupBy::Description, &GroupKey::None),
            "No description"
        );
        assert_eq!(n.name(GroupBy::Billable, &GroupKey::Bool(true)), "Billable");
        assert_eq!(
            n.name(GroupBy::Billable, &GroupKey::Bool(false)),
            "Non-billable"
        );
        assert_eq!(
            n.name(GroupBy::Description, &GroupKey::Text("fix".into())),
            "fix"
        );
        assert_eq!(
            n.name(GroupBy::Day, &GroupKey::Date(d(2026, 11, 9))),
            "9.11.2026"
        );
        // Project colors for project and task groups only.
        assert_eq!(
            n.color(GroupBy::Project, &GroupKey::Id(web)),
            Color::from_rgb_u8(255, 0, 0)
        );
        assert_eq!(
            n.color(GroupBy::Task, &GroupKey::Id(login)),
            Color::from_rgb_u8(255, 0, 0)
        );
        assert_eq!(
            n.color(GroupBy::Client, &GroupKey::Id(f.clients[0].id))
                .alpha(),
            0
        );
        assert_eq!(n.color(GroupBy::Project, &GroupKey::None).alpha(), 0);
    }

    #[test]
    fn flatten_rows_lists_groups_with_children_and_the_total_last() {
        let f = fx();
        let web = f.catalog.projects[0].id;
        let login = f.catalog.tasks[0].id;
        let result = ReportResult {
            total_seconds: 3 * 3600 + 1800,
            total_cost_cents: 15_000,
            groups: vec![
                row(
                    GroupKey::Id(web),
                    3 * 3600,
                    15_000,
                    vec![
                        row(GroupKey::Id(login), 2 * 3600, 10_000, vec![]),
                        row(GroupKey::None, 3600, 5_000, vec![]),
                    ],
                ),
                row(GroupKey::None, 1800, 0, vec![]),
            ],
            buckets: vec![],
        };
        let prefs = FormatPrefs::default();
        let n = f.names();
        let rows = flatten_rows(&result, GroupBy::Project, Some(GroupBy::Task), &n, &prefs);
        let got: Vec<(i32, &str, &str, &str)> = rows
            .iter()
            .map(|r| {
                (
                    r.level,
                    r.label.as_str(),
                    r.duration.as_str(),
                    r.cost.as_str(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (0, "Website", "3:00", "$150.00"),
                (1, "Website › Login", "2:00", "$100.00"),
                (1, "No task", "1:00", "$50.00"),
                (0, "No project", "0:30", ""),
                (0, "Total", "3:30", "$150.00"),
            ]
        );
        // Project and task groups get the project color, the "none" group and the total none.
        assert_eq!(rows[0].color, Color::from_rgb_u8(255, 0, 0));
        assert_eq!(rows[1].color, Color::from_rgb_u8(255, 0, 0));
        assert_eq!(rows[2].color.alpha(), 0);
        assert_eq!(rows[3].color.alpha(), 0);
        assert_eq!(rows[4].color.alpha(), 0);

        // No groups: only the total.
        let empty = ReportResult {
            total_seconds: 0,
            total_cost_cents: 0,
            groups: vec![],
            buckets: vec![],
        };
        let rows = flatten_rows(&empty, GroupBy::Project, None, &n, &prefs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Total");
        assert_eq!(rows[0].cost, "");
    }

    #[test]
    fn view_has_bars_by_interval_and_a_pie_with_other() {
        let f = fx();
        let n = f.names();
        let prefs = FormatPrefs::default();
        let web = f.catalog.projects[0].id;
        let groups: Vec<GroupRow> = (0..10)
            .map(|i| {
                let key = if i == 0 {
                    GroupKey::Id(web)
                } else {
                    GroupKey::Text(format!("g{i}"))
                };
                row(key, 3600 * (10 - i), 0, vec![])
            })
            .collect();
        let total: i64 = groups.iter().map(|g| g.seconds).sum();
        let result = ReportResult {
            total_seconds: total,
            total_cost_cents: 0,
            groups,
            buckets: vec![
                tt_domain::report::Bucket {
                    start: d(2026, 11, 2),
                    seconds: 3600,
                },
                tt_domain::report::Bucket {
                    start: d(2026, 11, 9),
                    seconds: 7200,
                },
            ],
        };
        let v = report_view(&result, GroupBy::Project, None, Interval::Week, &n, &prefs);
        let labels: Vec<_> = v.bars.iter().map(|b| b.label.to_string()).collect();
        assert_eq!(labels, ["02.11", "09.11"]);
        assert_eq!(v.bars[1].value, 1.0);
        assert_eq!(v.bars[0].caption, "2026-11-02 – 2026-11-08 · 1:00");
        assert_eq!(v.pie.len(), PIE_SLICES + 1);
        assert_eq!(v.pie.last().unwrap().label, "Other");
        assert_eq!(v.pie[0].color, Color::from_rgb_u8(255, 0, 0));
        assert_eq!(v.cost, "");
        assert_eq!(v.total, format_duration(total, &prefs));

        let m = report_view(&result, GroupBy::Project, None, Interval::Month, &n, &prefs);
        assert_eq!(m.bars[0].label, "M11");
        assert_eq!(m.bars[0].caption, "M11 2026 · 1:00");
    }

    #[test]
    fn filters_list_all_items_by_name_with_the_selection_text() {
        let f = fx();
        let selected = vec![f.catalog.projects[0].id, f.catalog.projects[1].id];
        let v = project_filter(&f.catalog, &selected);
        let names: Vec<String> = v.options.iter().map(|o| o.name.to_string()).collect();
        assert_eq!(names, ["API", "Website"]);
        assert_eq!(v.text, "API, Website");
        assert!(v.options.iter().all(|o| o.selected));
        let v = task_filter(&f.catalog, &[]);
        assert_eq!(v.options.row_data(0).unwrap().name, "Website › Login");
        assert_eq!(v.text, "");
        let v = client_filter(&f.clients, &[f.clients[0].id]);
        assert_eq!(v.text, "Acme");

        let mut s = ReportState::default();
        s.project_ids = vec![selected[0], new_id()];
        s.client_ids = vec![new_id()];
        s.retain_existing(&f.catalog, &f.clients);
        assert_eq!(s.project_ids, [selected[0]]);
        assert!(s.client_ids.is_empty());
    }
}
