//! Month grid of the date picker.

use chrono::{Datelike, Days, NaiveDate, Weekday};
use slint::{ModelRc, SharedString, VecModel};

use crate::{CalendarDay, CalendarMonth};

/// ISO date as used by the date pickers.
pub const ISO_DATE: &str = "%Y-%m-%d";

pub fn iso(d: NaiveDate) -> SharedString {
    d.format(ISO_DATE).to_string().into()
}

pub fn parse_iso(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, ISO_DATE).ok()
}

/// One day of a month grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayCell {
    pub date: NaiveDate,
    pub in_month: bool,
    pub is_today: bool,
}

/// Six weeks × seven days covering `month` of `year` (1 … 12), each week starting on
/// `week_start`; days of the neighbouring months are marked `in_month: false`.
/// An invalid month yields an empty grid.
pub fn month_grid(year: i32, month: u32, week_start: Weekday, today: NaiveDate) -> Vec<DayCell> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Vec::new();
    };
    let lead = first.weekday().days_since(week_start);
    let Some(start) = first.checked_sub_days(Days::new(lead.into())) else {
        return Vec::new();
    };
    start
        .iter_days()
        .take(42)
        .map(|date| DayCell {
            date,
            in_month: date.month() == month && date.year() == year,
            is_today: date == today,
        })
        .collect()
}

/// First day of the month `delta` months after the month of `d`.
pub fn add_months(d: NaiveDate, delta: i32) -> Option<NaiveDate> {
    let index = d.year() * 12 + d.month0() as i32 + delta;
    NaiveDate::from_ymd_opt(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1)
}

/// Picker view of the month of `anchor` (else of `today`) moved by `delta` months.
pub fn calendar_month(
    anchor: Option<NaiveDate>,
    delta: i32,
    week_start: Weekday,
    today: NaiveDate,
) -> CalendarMonth {
    let Some(first) = add_months(anchor.unwrap_or(today), delta) else {
        return CalendarMonth::default();
    };
    let days: Vec<CalendarDay> = month_grid(first.year(), first.month(), week_start, today)
        .into_iter()
        .map(|c| CalendarDay {
            date: iso(c.date),
            day: c.date.day() as i32,
            weekday: c.date.weekday().num_days_from_monday() as i32,
            in_month: c.in_month,
            is_today: c.is_today,
        })
        .collect();
    CalendarMonth {
        year: first.year(),
        month: first.month() as i32,
        first: iso(first),
        days: ModelRc::new(VecModel::from(days)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn grid_has_six_weeks_starting_on_monday() {
        // November 2026 starts on a Sunday.
        let g = month_grid(2026, 11, Weekday::Mon, date(2026, 11, 9));
        assert_eq!(g.len(), 42);
        assert_eq!(g[0].date, date(2026, 10, 26));
        assert!(!g[0].in_month);
        assert_eq!(g[6].date, date(2026, 11, 1));
        assert!(g[6].in_month);
        assert_eq!(g[35].date, date(2026, 11, 30));
        assert_eq!(g[41].date, date(2026, 12, 6));
        assert!(!g[41].in_month);
        assert!(
            g.iter()
                .all(|c| c.is_today == (c.date == date(2026, 11, 9)))
        );
        assert_eq!(g.iter().filter(|c| c.is_today).count(), 1);
        assert!(g.chunks(7).all(|w| w[0].date.weekday() == Weekday::Mon));
        assert!(
            g.windows(2)
                .all(|w| w[1].date == w[0].date.succ_opt().unwrap())
        );
    }

    #[test]
    fn grid_starting_on_sunday() {
        let g = month_grid(2026, 11, Weekday::Sun, date(2000, 1, 1));
        // The 1st falls on the week start: first cell, no days of October.
        assert_eq!(g[0].date, date(2026, 11, 1));
        assert!(g[..30].iter().all(|c| c.in_month));
        assert!(g[30..].iter().all(|c| !c.in_month));
        assert_eq!(g[41].date, date(2026, 12, 12));
        assert!(g.iter().all(|c| !c.is_today));
        assert!(g.chunks(7).all(|w| w[0].date.weekday() == Weekday::Sun));
    }

    #[test]
    fn month_starting_on_week_start_with_monday() {
        // June 2026 starts on a Monday.
        let g = month_grid(2026, 6, Weekday::Mon, date(2026, 6, 30));
        assert_eq!(g[0].date, date(2026, 6, 1));
        assert!(g[0].in_month);
        assert_eq!(g[29].date, date(2026, 6, 30));
        assert!(g[29].is_today);
        assert!(!g[30].in_month);
    }

    #[test]
    fn february_of_a_leap_year() {
        // 2028-02-01 is a Tuesday; February 2028 has 29 days.
        let g = month_grid(2028, 2, Weekday::Mon, date(2028, 2, 29));
        assert_eq!(g.len(), 42);
        assert_eq!(g[0].date, date(2028, 1, 31));
        assert_eq!(g.iter().filter(|c| c.in_month).count(), 29);
        let last = g.iter().rposition(|c| c.in_month).unwrap();
        assert_eq!(g[last].date, date(2028, 2, 29));
        assert!(g[last].is_today);
        assert_eq!(g[last + 1].date, date(2028, 3, 1));
    }

    #[test]
    fn calendar_month_maps_cells_and_moves() {
        use slint::Model;
        let today = date(2026, 11, 9);
        let m = calendar_month(Some(date(2026, 12, 31)), -1, Weekday::Sun, today);
        assert_eq!(
            (m.year, m.month, m.first.as_str()),
            (2026, 11, "2026-11-01")
        );
        assert_eq!(m.days.row_count(), 42);
        let first = m.days.row_data(0).unwrap();
        assert_eq!(first.date, "2026-11-01");
        assert_eq!((first.day, first.weekday, first.in_month), (1, 6, true));
        let t = m.days.iter().find(|d| d.is_today).unwrap();
        assert_eq!((t.date.as_str(), t.weekday), ("2026-11-09", 0));
        // No anchor: today's month.
        let m = calendar_month(None, 0, Weekday::Mon, today);
        assert_eq!(m.first, "2026-11-01");
        assert_eq!(parse_iso("2026-11-09"), Some(today));
        assert_eq!(parse_iso("nope"), None);
        assert_eq!(iso(today), "2026-11-09");
    }

    #[test]
    fn invalid_month_is_empty() {
        assert!(month_grid(2026, 13, Weekday::Mon, date(2026, 1, 1)).is_empty());
        assert!(month_grid(2026, 0, Weekday::Mon, date(2026, 1, 1)).is_empty());
    }

    #[test]
    fn add_months_moves_to_first_of_month() {
        assert_eq!(add_months(date(2026, 11, 30), 0), Some(date(2026, 11, 1)));
        assert_eq!(add_months(date(2026, 11, 30), 1), Some(date(2026, 12, 1)));
        assert_eq!(add_months(date(2026, 12, 31), 1), Some(date(2027, 1, 1)));
        assert_eq!(add_months(date(2026, 1, 15), -1), Some(date(2025, 12, 1)));
        assert_eq!(add_months(date(2026, 3, 31), -13), Some(date(2025, 2, 1)));
    }
}
