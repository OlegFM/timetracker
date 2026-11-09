//! Dashboard view models: bars, pie slices, activity cells and project totals.

use chrono::{Datelike, NaiveDate};
use slint::Color;
use tt_domain::{EntryTemplate, FormatPrefs, Id, format_date, format_duration, format_money};
use tt_services::Dashboard;

use crate::vm::pickers::{Catalog, hex_color, option_label};
use crate::{BarView, HeatCell, PieSlice, ProjectTotalView, RecentView};

/// Between the parts of a chart caption.
const CAPTION_SEPARATOR: &str = " · ";

/// Center and radius of the pie in its 100 x 100 box.
const PIE_CENTER: f64 = 50.0;
const PIE_RADIUS: f64 = 48.0;

/// Texts the dashboard needs from the UI language.
#[derive(Debug, Clone, Default)]
pub struct Labels {
    /// Short weekday names, Monday first.
    pub weekdays: [String; 7],
    /// Time without a project.
    pub no_project: String,
}

impl Labels {
    fn weekday(&self, d: NaiveDate) -> &str {
        &self.weekdays[d.weekday().num_days_from_monday() as usize]
    }
}

/// Everything the dashboard shows.
#[derive(Debug, Clone, Default)]
pub struct DashboardView {
    pub recent: Vec<RecentView>,
    pub last7: Vec<BarView>,
    pub last7_total: String,
    pub heatmap: Vec<HeatCell>,
    pub week: Vec<BarView>,
    pub week_projects: Vec<ProjectTotalView>,
    pub week_total: String,
    /// Empty when nothing is billable.
    pub week_cost: String,
    pub pie: Vec<PieSlice>,
}

/// Each value as a fraction of the largest (0..=1); all zeros (or negatives) give 0.
pub fn bars_normalized(values: &[i64]) -> Vec<f32> {
    let max = values.iter().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|&v| {
            if max > 0 && v > 0 {
                v as f32 / max as f32
            } else {
                0.0
            }
        })
        .collect()
}

/// Activity level 0..=4: 0 without time, else the quarter of `max_ms` the time reaches.
pub fn heat_level(ms: i64, max_ms: i64) -> i32 {
    if ms <= 0 {
        0
    } else if max_ms <= 0 {
        4
    } else {
        // ceil(4 * ms / max), at least 1 for any time and at most 4.
        ((ms.saturating_mul(4) + max_ms - 1) / max_ms).clamp(1, 4) as i32
    }
}

/// Point of the pie circle at `degrees` clockwise from 12 o'clock.
fn pie_point(degrees: f64) -> (f64, f64) {
    let rad = degrees.to_radians();
    (
        PIE_CENTER + PIE_RADIUS * rad.sin(),
        PIE_CENTER - PIE_RADIUS * rad.cos(),
    )
}

/// SVG path (100 x 100 box) of the slice: center, line to the start, arc to the end.
fn slice_path(start: f64, sweep: f64) -> String {
    let (x0, y0) = pie_point(start);
    if sweep >= 360.0 - 1e-6 {
        // One arc cannot start and end at the same point: two half circles.
        let (x1, y1) = pie_point(start + 180.0);
        return format!(
            "M {x0:.3} {y0:.3} A {PIE_RADIUS} {PIE_RADIUS} 0 1 1 {x1:.3} {y1:.3} \
             A {PIE_RADIUS} {PIE_RADIUS} 0 1 1 {x0:.3} {y0:.3} Z"
        );
    }
    let (x1, y1) = pie_point(start + sweep);
    let large = u8::from(sweep > 180.0);
    format!(
        "M {PIE_CENTER} {PIE_CENTER} L {x0:.3} {y0:.3} \
         A {PIE_RADIUS} {PIE_RADIUS} 0 {large} 1 {x1:.3} {y1:.3} Z"
    )
}

/// Slices of a pie in order, starting at 12 o'clock and going clockwise; zero (or
/// negative) values are skipped. The sweeps add up to 360; a single value is a full circle.
/// The caption of a slice is its label.
pub fn pie_slices(values: &[(String, i64, Color)]) -> Vec<PieSlice> {
    let items: Vec<&(String, i64, Color)> = values.iter().filter(|v| v.1 > 0).collect();
    let total: i64 = items.iter().map(|v| v.1).sum();
    let mut start = 0.0_f64;
    let mut acc = 0_i64;
    let last = items.len().saturating_sub(1);
    items
        .iter()
        .enumerate()
        .map(|(i, (label, ms, color))| {
            acc += ms;
            // From the running sum, so that rounding does not add up; the last one
            // closes the circle exactly.
            let end = if i == last {
                360.0
            } else {
                360.0 * acc as f64 / total as f64
            };
            let slice = PieSlice {
                start_angle: start as f32,
                sweep: (end - start) as f32,
                color: *color,
                label: label.as_str().into(),
                caption: label.as_str().into(),
                path: slice_path(start, end - start).into(),
            };
            start = end;
            slice
        })
        .collect()
}

fn day_caption(d: NaiveDate, ms: i64, prefs: &FormatPrefs) -> String {
    format!(
        "{}{CAPTION_SEPARATOR}{}",
        format_date(d, prefs.date),
        format_duration(ms / 1000, prefs)
    )
}

/// One bar per day, labeled with the weekday.
fn bar_views(days: &[(NaiveDate, i64)], labels: &Labels, prefs: &FormatPrefs) -> Vec<BarView> {
    let ms: Vec<i64> = days.iter().map(|d| d.1).collect();
    days.iter()
        .zip(bars_normalized(&ms))
        .map(|(&(d, ms), value)| BarView {
            label: labels.weekday(d).into(),
            value,
            caption: day_caption(d, ms, prefs).into(),
        })
        .collect()
}

/// The activity cells in the order of `days`.
fn heat_cells(days: &[(NaiveDate, i64)], prefs: &FormatPrefs) -> Vec<HeatCell> {
    let max = days.iter().map(|d| d.1).max().unwrap_or(0);
    days.iter()
        .map(|&(d, ms)| HeatCell {
            level: heat_level(ms, max),
            caption: day_caption(d, ms, prefs).into(),
        })
        .collect()
}

/// Money text; empty for nothing.
fn money(cents: i64, prefs: &FormatPrefs) -> String {
    if cents > 0 {
        format_money(cents, prefs)
    } else {
        String::new()
    }
}

/// A recently tracked task: its description and project (with task) and color.
pub fn recent_view(t: &EntryTemplate, c: &Catalog) -> RecentView {
    let project = t.project_id.and_then(|id| c.project(id));
    let task = t.task_id.and_then(|id| c.task(id));
    RecentView {
        description: t.description.trim().into(),
        project_label: project
            .map_or(String::new(), |p| option_label(p, task))
            .into(),
        project_color: project.map_or_else(Color::default, |p| hex_color(&p.color)),
    }
}

/// Rows of the week by project with their milliseconds; a transparent color is "no
/// project".
fn project_totals(
    by_project: &[(Option<Id>, i64, i64)],
    c: &Catalog,
    labels: &Labels,
    prefs: &FormatPrefs,
) -> Vec<(ProjectTotalView, i64)> {
    by_project
        .iter()
        .map(|&(id, ms, cost)| {
            let project = id.and_then(|id| c.project(id));
            let row = ProjectTotalView {
                label: project
                    .map_or(labels.no_project.as_str(), |p| p.name.as_str())
                    .into(),
                color: project.map_or_else(Color::default, |p| hex_color(&p.color)),
                duration: format_duration(ms / 1000, prefs).into(),
                cost: money(cost, prefs).into(),
            };
            (row, ms)
        })
        .collect()
}

/// Maps the loaded dashboard to what the screen shows.
pub fn dashboard_view(
    d: &Dashboard,
    c: &Catalog,
    labels: &Labels,
    prefs: &FormatPrefs,
) -> DashboardView {
    let totals = project_totals(&d.week_by_project, c, labels, prefs);
    let shown: Vec<&(ProjectTotalView, i64)> = totals.iter().filter(|t| t.1 > 0).collect();
    let pie_input: Vec<(String, i64, Color)> = shown
        .iter()
        .map(|(row, ms)| (row.label.to_string(), *ms, row.color))
        .collect();
    let mut pie = pie_slices(&pie_input);
    let sum: i64 = shown.iter().map(|t| t.1).sum::<i64>().max(1);
    for (slice, (row, ms)) in pie.iter_mut().zip(shown) {
        let percent = (ms * 100 + sum / 2) / sum;
        slice.caption = format!(
            "{}{CAPTION_SEPARATOR}{}{CAPTION_SEPARATOR}{percent} %",
            row.label, row.duration
        )
        .into();
    }
    let last7_ms: i64 = d.last7.iter().map(|x| x.1).sum();
    DashboardView {
        recent: d.recent.iter().map(|t| recent_view(t, c)).collect(),
        last7: bar_views(&d.last7, labels, prefs),
        last7_total: format_duration(last7_ms / 1000, prefs),
        heatmap: heat_cells(&d.heatmap, prefs),
        week: bar_views(&d.week_by_day, labels, prefs),
        week_projects: totals.into_iter().map(|t| t.0).collect(),
        week_total: format_duration(d.week_total_ms / 1000, prefs),
        week_cost: money(d.week_cost_cents, prefs),
        pie,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::pickers::tests::{project, task};
    use tt_domain::{DateFormat, IntervalFormat, new_id};

    fn item(label: &str, ms: i64) -> (String, i64, Color) {
        (label.to_owned(), ms, Color::from_rgb_u8(1, 2, 3))
    }

    #[test]
    fn pie_slices_cover_the_circle_in_order_and_skip_zeros() {
        let slices = pie_slices(&[item("a", 1), item("zero", 0), item("b", 2), item("c", 1)]);
        let labels: Vec<&str> = slices.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["a", "b", "c"]);
        let total: f32 = slices.iter().map(|s| s.sweep).sum();
        assert!((total - 360.0).abs() < 1e-3, "{total}");
        assert_eq!(slices[0].start_angle, 0.0);
        assert!((slices[0].sweep - 90.0).abs() < 1e-3);
        assert!((slices[1].start_angle - 90.0).abs() < 1e-3);
        assert!((slices[1].sweep - 180.0).abs() < 1e-3);
        assert!((slices[2].start_angle - 270.0).abs() < 1e-3);
        assert!((slices[2].start_angle + slices[2].sweep - 360.0).abs() < 1e-3);
    }

    #[test]
    fn pie_slices_edge_cases() {
        assert!(pie_slices(&[]).is_empty());
        assert!(pie_slices(&[item("a", 0)]).is_empty());

        let one = pie_slices(&[item("a", 5)]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].sweep, 360.0);
        // A full circle is two half arcs (one arc cannot start and end at one point).
        assert_eq!(one[0].path.matches('A').count(), 2);
        assert!(one[0].path.ends_with('Z'));
    }

    #[test]
    fn pie_slice_paths_are_arcs_from_the_center() {
        let slices = pie_slices(&[item("a", 1), item("b", 3)]);
        // A quarter: from 12 o'clock (50, 2) clockwise to 3 o'clock (98, 50), small arc.
        assert_eq!(
            slices[0].path,
            "M 50 50 L 50.000 2.000 A 48 48 0 0 1 98.000 50.000 Z"
        );
        // Three quarters: the large-arc flag is set.
        assert!(
            slices[1].path.contains("A 48 48 0 1 1"),
            "{}",
            slices[1].path
        );
    }

    #[test]
    fn heat_level_quartiles_of_the_maximum() {
        assert_eq!(heat_level(0, 100), 0);
        assert_eq!(heat_level(0, 0), 0);
        assert_eq!(heat_level(1, 100), 1);
        assert_eq!(heat_level(25, 100), 1);
        assert_eq!(heat_level(26, 100), 2);
        assert_eq!(heat_level(50, 100), 2);
        assert_eq!(heat_level(51, 100), 3);
        assert_eq!(heat_level(75, 100), 3);
        assert_eq!(heat_level(76, 100), 4);
        assert_eq!(heat_level(100, 100), 4);
        // Time that exists while the maximum is unknown still shows.
        assert_eq!(heat_level(5, 0), 4);
    }

    #[test]
    fn bars_are_fractions_of_the_maximum() {
        assert_eq!(bars_normalized(&[1, 2, 4]), [0.25, 0.5, 1.0]);
        assert_eq!(bars_normalized(&[0, 0]), [0.0, 0.0]);
        assert!(bars_normalized(&[]).is_empty());
    }

    fn labels() -> Labels {
        Labels {
            weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(String::from),
            no_project: "No project".into(),
        }
    }

    fn prefs() -> FormatPrefs {
        FormatPrefs {
            date: DateFormat::HyphenYYYYMMDD,
            interval: IntervalFormat::HoursMinutesColon,
            hour_suffix: "h".into(),
            minute_suffix: "min".into(),
            ..tt_services::Settings::default().format
        }
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn dashboard_view_maps_every_card() {
        const H: i64 = 3_600_000;
        let (web, api) = (project("Website", "#ff0000"), project("API", "#00ff00"));
        let login = task(&web, "Login");
        let catalog = Catalog {
            projects: vec![web.clone(), api.clone()],
            tasks: vec![login.clone()],
            tags: vec![],
        };
        let d = Dashboard {
            recent: vec![
                EntryTemplate {
                    description: " Fix ".into(),
                    project_id: Some(web.id),
                    task_id: Some(login.id),
                    ..EntryTemplate::default()
                },
                EntryTemplate::default(),
            ],
            last7: (9..16)
                .map(|n| (day(n), if n == 10 { 2 * H } else { 0 }))
                .collect(),
            heatmap: (0..84)
                .map(|i| {
                    (
                        day(1) + chrono::Days::new(i),
                        if i == 3 { 4 * H } else { H },
                    )
                })
                .collect(),
            week_by_day: vec![(day(9), H), (day(10), 3 * H), (day(11), 0)],
            week_by_project: vec![
                (Some(web.id), 3 * H, 30_000),
                (Some(api.id), H, 0),
                (None, 0, 0),
                (Some(new_id()), H, 0),
            ],
            week_total_ms: 4 * H,
            week_cost_cents: 30_000,
        };
        let v = dashboard_view(&d, &catalog, &labels(), &prefs());

        assert_eq!(v.recent[0].description, "Fix");
        assert_eq!(v.recent[0].project_label, "Website › Login");
        assert_eq!(v.recent[0].project_color, hex_color("#ff0000"));
        assert_eq!(v.recent[1].project_label, "");

        assert_eq!(v.last7.len(), 7);
        assert_eq!(v.last7[1].label, "Tue");
        assert_eq!(v.last7[1].value, 1.0);
        assert_eq!(v.last7[1].caption, "2026-11-10 · 2:00");
        assert_eq!(v.last7_total, "2:00");

        assert_eq!(v.heatmap.len(), 84);
        assert_eq!((v.heatmap[3].level, v.heatmap[0].level), (4, 1));

        assert_eq!(v.week[1].value, 1.0);
        assert!((v.week[0].value - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(v.week_total, "4:00");
        assert_eq!(v.week_cost, "$300.00");

        let rows: Vec<(&str, &str)> = v
            .week_projects
            .iter()
            .map(|r| (r.label.as_str(), r.cost.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Website", "$300.00"),
                ("API", ""),
                ("No project", ""),
                ("No project", "")
            ]
        );
        assert_eq!(v.week_projects[2].color.alpha(), 0);

        // The pie skips the zero row and its captions stay with the right slices.
        let captions: Vec<&str> = v.pie.iter().map(|s| s.caption.as_str()).collect();
        assert_eq!(
            captions,
            [
                "Website · 3:00 · 60 %",
                "API · 1:00 · 20 %",
                "No project · 1:00 · 20 %"
            ]
        );
    }
}
