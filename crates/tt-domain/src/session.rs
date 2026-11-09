use chrono::{Days, NaiveDate};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::days::start_of_local_day;
use crate::ids::{Id, Millis};
use crate::model::TimeEntry;

/// Fields copied from an entry when starting the next one in a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryTemplate {
    pub description: String,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub tag_ids: Vec<Id>,
    pub billable: bool,
}

impl EntryTemplate {
    pub fn from_entry(e: &TimeEntry) -> Self {
        Self {
            description: e.description.clone(),
            project_id: e.project_id,
            task_id: e.task_id,
            tag_ids: e.tag_ids.clone(),
            billable: e.billable,
        }
    }
}

/// Total tracked time (ms) of the entries; running entries count up to `now`.
pub fn session_total(entries: &[TimeEntry], now: Millis) -> i64 {
    entries.iter().map(|e| e.duration_ms(now)).sum()
}

/// Tracked time (ms) falling inside local day `day`; entries are clipped to the day.
pub fn day_total(entries: &[TimeEntry], day: NaiveDate, now: Millis, tz: Tz) -> i64 {
    let lo = start_of_local_day(day, tz);
    let next = day.checked_add_days(Days::new(1)).expect("date in range");
    let hi = start_of_local_day(next, tz);
    entries
        .iter()
        .map(|e| (e.end.unwrap_or(now).min(hi) - e.start.max(lo)).max(0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_id;
    use chrono::TimeZone;
    use chrono_tz::Europe::Moscow;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;

    fn entry(start: Millis, end: Option<Millis>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: "work".into(),
            start,
            end,
            billable: true,
            project_id: Some(new_id()),
            task_id: None,
            tag_ids: vec![new_id()],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn at(d: u32, h: u32, m: u32) -> Millis {
        let naive = NaiveDate::from_ymd_opt(2026, 11, d)
            .unwrap()
            .and_hms_opt(h, m, 0)
            .unwrap();
        Moscow
            .from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn template_copies_entry_fields() {
        let e = entry(0, Some(HOUR));
        let t = EntryTemplate::from_entry(&e);
        assert_eq!(t.description, e.description);
        assert_eq!(t.project_id, e.project_id);
        assert_eq!(t.task_id, e.task_id);
        assert_eq!(t.tag_ids, e.tag_ids);
        assert!(t.billable);
    }

    #[test]
    fn template_json_round_trip() {
        let t = EntryTemplate::from_entry(&entry(0, Some(HOUR)));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<EntryTemplate>(&json).unwrap(), t);
    }

    #[test]
    fn session_total_counts_running_entry_until_now() {
        let entries = [
            entry(at(9, 10, 0), Some(at(9, 10, 30))),
            entry(at(9, 11, 0), None),
        ];
        assert_eq!(session_total(&entries, at(9, 11, 15)), 45 * MIN);
    }

    #[test]
    fn day_total_counts_only_own_day() {
        let entries = [
            entry(at(9, 10, 0), Some(at(9, 11, 0))),
            entry(at(10, 10, 0), Some(at(10, 10, 30))),
        ];
        let now = at(11, 0, 0);
        let d9 = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let d10 = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        assert_eq!(day_total(&entries, d9, now, Moscow), HOUR);
        assert_eq!(day_total(&entries, d10, now, Moscow), 30 * MIN);
    }

    #[test]
    fn day_total_clips_entry_crossing_midnight() {
        let entries = [entry(at(9, 23, 0), Some(at(10, 1, 0)))];
        let now = at(11, 0, 0);
        let d9 = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let d10 = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        assert_eq!(day_total(&entries, d9, now, Moscow), HOUR);
        assert_eq!(day_total(&entries, d10, now, Moscow), HOUR);
    }

    #[test]
    fn day_total_running_entry_until_now() {
        let entries = [entry(at(9, 23, 0), None)];
        let d10 = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        assert_eq!(day_total(&entries, d10, at(10, 0, 30), Moscow), 30 * MIN);
    }
}
