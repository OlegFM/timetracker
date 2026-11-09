//! Report runs and the dashboard summary.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, Days, NaiveDate};
use tt_domain::{
    EntryTemplate, GroupBy, GroupKey, Id, Interval, Millis, Project, ReportContext, ReportFilter,
    ReportResult, Task, TimeEntry, aggregate, day_total, local_date, start_of_local_day,
};
use tt_storage::{Db, catalog, entries};

use crate::error::Result;
use crate::settings::Settings;

/// Projects and tasks by id (tasks of all projects).
pub(crate) fn catalog_maps(db: &Db) -> Result<(HashMap<Id, Project>, HashMap<Id, Task>)> {
    let projects = catalog::list_projects(db.conn())?;
    let mut tasks = HashMap::new();
    for p in &projects {
        for t in catalog::list_tasks(db.conn(), p.id)? {
            tasks.insert(t.id, t);
        }
    }
    Ok((projects.into_iter().map(|p| (p.id, p)).collect(), tasks))
}

fn context<'a>(
    projects: &'a HashMap<Id, Project>,
    tasks: &'a HashMap<Id, Task>,
    settings: &Settings,
    now: Millis,
) -> ReportContext<'a> {
    ReportContext {
        projects,
        tasks,
        default_rate: settings.default_rate,
        tz: settings.format.tz,
        week_start: settings.format.week_start,
        now,
    }
}

pub struct ReportService;

impl ReportService {
    pub fn run(
        db: &Db,
        filter: &ReportFilter,
        g1: GroupBy,
        g2: Option<GroupBy>,
        interval: Interval,
        settings: &Settings,
        now: Millis,
    ) -> Result<ReportResult> {
        let list = entries::entries_in_range(db.conn(), filter.start, filter.end)?;
        let (projects, tasks) = catalog_maps(db)?;
        let ctx = context(&projects, &tasks, settings, now);
        Ok(aggregate(&list, filter, g1, g2, interval, &ctx))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dashboard {
    pub recent: Vec<EntryTemplate>,
    /// 7 days ending today, oldest first; ms.
    pub last7: Vec<(NaiveDate, i64)>,
    /// 84 days: 12 full weeks, the last one being the current week; ms.
    pub heatmap: Vec<(NaiveDate, i64)>,
    pub week_by_day: Vec<(NaiveDate, i64)>,
    /// `(project, ms, cost cents)`, in report order.
    pub week_by_project: Vec<(Option<Id>, i64, i64)>,
    pub week_total_ms: i64,
    pub week_cost_cents: i64,
}

pub struct DashboardService;

const RECENT_LIMIT: usize = 6;
const RECENT_SCAN: u32 = 200;

impl DashboardService {
    pub fn load(db: &Db, settings: &Settings, now: Millis) -> Result<Dashboard> {
        let tz = settings.format.tz;
        let today = local_date(now, tz);
        let back = (today.weekday().num_days_from_monday() + 7
            - settings.format.week_start.num_days_from_monday())
            % 7;
        let week_start = today - Days::new(u64::from(back));
        let heat_start = week_start - Days::new(77);
        let week_end = week_start + Days::new(7);

        let recent = recent_templates(&entries::recent_entries(db.conn(), RECENT_SCAN)?);

        let lo = start_of_local_day(heat_start, tz);
        let hi = start_of_local_day(week_end.max(today + Days::new(1)), tz);
        let list = entries::entries_in_range(db.conn(), lo, hi)?;
        let totals = |from: NaiveDate, days: u64| -> Vec<(NaiveDate, i64)> {
            (0..days)
                .map(|i| {
                    let d = from + Days::new(i);
                    (d, day_total(&list, d, now, tz))
                })
                .collect()
        };

        let (projects, tasks) = catalog_maps(db)?;
        let ctx = context(&projects, &tasks, settings, now);
        let filter = ReportFilter {
            start: start_of_local_day(week_start, tz),
            end: start_of_local_day(week_end, tz),
            project_ids: vec![],
            task_ids: vec![],
            client_ids: vec![],
            tag_ids: vec![],
            tag_match: tt_domain::TagMatch::Any,
            billable: None,
            rounding: None,
        };
        let week = aggregate(&list, &filter, GroupBy::Project, None, Interval::Day, &ctx);

        Ok(Dashboard {
            recent,
            last7: totals(today - Days::new(6), 7),
            heatmap: totals(heat_start, 84),
            week_by_day: totals(week_start, 7),
            week_by_project: week
                .groups
                .iter()
                .map(|g| {
                    let id = match g.key {
                        GroupKey::Id(id) => Some(id),
                        _ => None,
                    };
                    (id, g.seconds * 1000, g.cost_cents)
                })
                .collect(),
            week_total_ms: week.total_seconds * 1000,
            week_cost_cents: week.total_cost_cents,
        })
    }
}

fn recent_templates(latest_first: &[TimeEntry]) -> Vec<EntryTemplate> {
    let mut seen = HashSet::new();
    latest_first
        .iter()
        .filter(|e| seen.insert((e.description.trim().to_owned(), e.project_id, e.task_id)))
        .take(RECENT_LIMIT)
        .map(EntryTemplate::from_entry)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Weekday};
    use tt_domain::{Tz, new_id};

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const MSK: Tz = Tz::Europe__Moscow;

    fn at(d: u32, h: u32, mi: u32) -> Millis {
        let naive = NaiveDate::from_ymd_opt(2026, 11, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap();
        MSK.from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn settings() -> Settings {
        let mut s = Settings::default();
        s.format.tz = MSK;
        s.format.week_start = Weekday::Mon;
        s.default_rate = Some(6000); // 60.00 per hour
        s
    }

    fn put(
        db: &Db,
        desc: &str,
        project: Option<Id>,
        start: Millis,
        end: Option<Millis>,
    ) -> TimeEntry {
        let e = TimeEntry {
            id: new_id(),
            session_id: None,
            description: desc.into(),
            start,
            end,
            billable: true,
            project_id: project,
            task_id: None,
            tag_ids: vec![],
            created_at: 0,
            updated_at: 0,
        };
        entries::insert_entry(db.conn(), &e).unwrap();
        e
    }

    #[test]
    fn dashboard_recent_unique_and_ordered() {
        let db = Db::open_in_memory().unwrap();
        put(&db, "a", None, at(2, 9, 0), Some(at(2, 10, 0)));
        put(&db, "b", None, at(3, 9, 0), Some(at(3, 10, 0)));
        put(&db, " a ", None, at(4, 9, 0), Some(at(4, 10, 0))); // same as "a"
        for i in 0..6 {
            put(
                &db,
                &format!("x{i}"),
                None,
                at(5, 8 + i, 0),
                Some(at(5, 8 + i, 30)),
            );
        }
        let d = DashboardService::load(&db, &settings(), at(9, 12, 0)).unwrap();
        let got: Vec<&str> = d.recent.iter().map(|t| t.description.as_str()).collect();
        assert_eq!(got, ["x5", "x4", "x3", "x2", "x1", "x0"]);

        put(&db, "a", None, at(6, 9, 0), Some(at(6, 10, 0)));
        let d = DashboardService::load(&db, &settings(), at(9, 12, 0)).unwrap();
        assert_eq!(d.recent[0].description, "a");
        assert_eq!(d.recent.len(), 6);
        assert_eq!(
            d.recent
                .iter()
                .filter(|t| t.description.trim() == "a")
                .count(),
            1
        );
    }

    #[test]
    fn dashboard_heatmap_has_84_days_aligned_to_week_start() {
        let db = Db::open_in_memory().unwrap();
        // Wednesday 2026-11-11; week starts Monday 2026-11-09.
        let now = at(11, 12, 0);
        put(&db, "w", None, at(10, 9, 0), Some(at(10, 10, 0)));
        let d = DashboardService::load(&db, &settings(), now).unwrap();
        assert_eq!(d.heatmap.len(), 84);
        assert_eq!(d.heatmap[0].0, day(11, 9) - Days::new(77));
        assert_eq!(d.heatmap[0].0.weekday(), Weekday::Mon);
        assert_eq!(d.heatmap[83].0, day(11, 15));
        assert_eq!(
            d.heatmap.iter().find(|x| x.0 == day(11, 10)).unwrap().1,
            HOUR
        );
        assert_eq!(d.heatmap[83].1, 0, "future day is empty");

        let mut s = settings();
        s.format.week_start = Weekday::Sun;
        let d = DashboardService::load(&db, &s, now).unwrap();
        assert_eq!(d.heatmap[0].0.weekday(), Weekday::Sun);
        assert_eq!(d.heatmap[83].0, day(11, 14));
        assert_eq!(d.last7.len(), 7);
        assert_eq!(d.last7[6].0, day(11, 11));
    }

    #[test]
    fn dashboard_week_totals() {
        let db = Db::open_in_memory().unwrap();
        let p = Project {
            id: new_id(),
            name: "P".into(),
            color: "#000000".into(),
            client_id: None,
            is_billable: true,
            billable_rate: Some(12000),
            estimated_time: None,
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        };
        catalog::insert_project(db.conn(), &p).unwrap();
        put(&db, "old", None, at(8, 9, 0), Some(at(8, 11, 0))); // previous week (Sunday)
        put(&db, "p", Some(p.id), at(9, 9, 0), Some(at(9, 10, 0)));
        put(&db, "n", None, at(10, 9, 0), Some(at(10, 9, 30)));
        put(&db, "run", None, at(11, 11, 0), None); // running until now
        let d = DashboardService::load(&db, &settings(), at(11, 11, 30)).unwrap();

        assert_eq!(d.week_total_ms, 2 * HOUR);
        assert_eq!(d.week_cost_cents, 12000 + 3000 + 3000);
        assert_eq!(
            d.week_by_day.iter().map(|x| x.1).collect::<Vec<_>>(),
            [HOUR, 30 * MIN, 30 * MIN, 0, 0, 0, 0]
        );
        assert_eq!(d.week_by_day[0].0, day(11, 9));
        assert_eq!(
            d.week_by_project,
            [(Some(p.id), HOUR, 12000), (None, HOUR, 6000)]
        );
        assert_eq!(d.last7[0].0, day(11, 5));
        assert_eq!(d.last7[3].1, 2 * HOUR, "Sunday the 8th");
    }
}
