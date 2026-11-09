use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::billing::{cost_cents, effective_rate};
use crate::days::local_date;
use crate::format::key_enum;
use crate::ids::{Id, Millis};
use crate::model::{Project, Task, TimeEntry};
use crate::rounding::{RoundingMode, rounded_duration_s};

key_enum! {
    TagMatch {
        Any => "any",
        All => "all",
    }
}

key_enum! {
    GroupBy {
        Project => "project",
        Task => "task",
        Client => "client",
        Tag => "tag",
        Billable => "billable",
        Description => "description",
        Day => "day",
    }
}

key_enum! {
    Interval {
        Day => "day",
        Week => "week",
        Month => "month",
    }
}

/// Report selection. An empty id vector means "no filter on that axis".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportFilter {
    /// Inclusive start (UTC ms).
    pub start: Millis,
    /// Exclusive end (UTC ms).
    pub end: Millis,
    pub project_ids: Vec<Id>,
    pub task_ids: Vec<Id>,
    pub client_ids: Vec<Id>,
    pub tag_ids: Vec<Id>,
    pub tag_match: TagMatch,
    pub billable: Option<bool>,
    /// `(mode, step in minutes)`.
    pub rounding: Option<(RoundingMode, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupKey {
    None,
    Id(Id),
    Bool(bool),
    Text(String),
    Date(NaiveDate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub key: GroupKey,
    pub seconds: i64,
    pub cost_cents: i64,
    pub children: Vec<GroupRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub start: NaiveDate,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportResult {
    pub total_seconds: i64,
    pub total_cost_cents: i64,
    pub groups: Vec<GroupRow>,
    pub buckets: Vec<Bucket>,
}

pub struct ReportContext<'a> {
    pub projects: &'a HashMap<Id, Project>,
    pub tasks: &'a HashMap<Id, Task>,
    pub default_rate: Option<i64>,
    pub tz: Tz,
    pub week_start: Weekday,
    pub now: Millis,
}

/// An entry that passed the filter, with its computed seconds and cost.
struct Item<'a> {
    entry: &'a TimeEntry,
    seconds: i64,
    cost: i64,
}

fn client_of(e: &TimeEntry, ctx: &ReportContext) -> Option<Id> {
    e.project_id
        .and_then(|p| ctx.projects.get(&p))
        .and_then(|p| p.client_id)
}

fn matches(e: &TimeEntry, f: &ReportFilter, ctx: &ReportContext) -> bool {
    let in_ids = |ids: &[Id], v: Option<Id>| ids.is_empty() || v.is_some_and(|v| ids.contains(&v));
    let tags_ok = f.tag_ids.is_empty()
        || match f.tag_match {
            TagMatch::Any => f.tag_ids.iter().any(|t| e.tag_ids.contains(t)),
            TagMatch::All => f.tag_ids.iter().all(|t| e.tag_ids.contains(t)),
        };
    f.start <= e.start
        && e.start < f.end
        && in_ids(&f.project_ids, e.project_id)
        && in_ids(&f.task_ids, e.task_id)
        && in_ids(&f.client_ids, client_of(e, ctx))
        && tags_ok
        && f.billable.is_none_or(|b| b == e.billable)
}

/// The entries a report over `filter` covers (period, projects, tasks, clients, tags,
/// billable), in input order. Rounding does not affect selection.
pub fn select_entries<'a>(
    entries: &'a [TimeEntry],
    filter: &ReportFilter,
    ctx: &ReportContext,
) -> Vec<&'a TimeEntry> {
    entries.iter().filter(|e| matches(e, filter, ctx)).collect()
}

fn keys(e: &TimeEntry, by: GroupBy, ctx: &ReportContext) -> Vec<GroupKey> {
    let opt = |v: Option<Id>| v.map_or(GroupKey::None, GroupKey::Id);
    match by {
        GroupBy::Project => vec![opt(e.project_id)],
        GroupBy::Task => vec![opt(e.task_id)],
        GroupBy::Client => vec![opt(client_of(e, ctx))],
        GroupBy::Tag if e.tag_ids.is_empty() => vec![GroupKey::None],
        GroupBy::Tag => e.tag_ids.iter().copied().map(GroupKey::Id).collect(),
        GroupBy::Billable => vec![GroupKey::Bool(e.billable)],
        GroupBy::Description => {
            let t = e.description.trim();
            vec![if t.is_empty() {
                GroupKey::None
            } else {
                GroupKey::Text(t.to_owned())
            }]
        }
        GroupBy::Day => vec![GroupKey::Date(local_date(e.start, ctx.tz))],
    }
}

fn group(
    items: &[&Item],
    by: GroupBy,
    next: Option<GroupBy>,
    ctx: &ReportContext,
) -> Vec<GroupRow> {
    let mut map: BTreeMap<GroupKey, Vec<&Item>> = BTreeMap::new();
    for &it in items {
        for k in keys(it.entry, by, ctx) {
            map.entry(k).or_default().push(it);
        }
    }
    let mut rows: Vec<GroupRow> = map
        .into_iter()
        .map(|(key, members)| GroupRow {
            key,
            seconds: members.iter().map(|i| i.seconds).sum(),
            cost_cents: members.iter().map(|i| i.cost).sum(),
            children: next.map_or_else(Vec::new, |n| group(&members, n, None, ctx)),
        })
        .collect();
    // Days stay chronological (BTreeMap order); other groups go by time, largest first. The
    // stable sort keeps key ties deterministic and "no value" last.
    if by != GroupBy::Day {
        rows.sort_by_key(|r| (r.key == GroupKey::None, std::cmp::Reverse(r.seconds)));
    }
    rows
}

fn bucket_start(d: NaiveDate, interval: Interval, week_start: Weekday) -> NaiveDate {
    match interval {
        Interval::Day => d,
        Interval::Week => {
            let back =
                (d.weekday().num_days_from_monday() + 7 - week_start.num_days_from_monday()) % 7;
            d - Days::new(u64::from(back))
        }
        Interval::Month => d.with_day(1).expect("day 1 exists"),
    }
}

fn next_bucket(d: NaiveDate, interval: Interval) -> NaiveDate {
    match interval {
        Interval::Day => d + Days::new(1),
        Interval::Week => d + Days::new(7),
        Interval::Month => d + Months::new(1),
    }
}

fn buckets(
    items: &[Item],
    f: &ReportFilter,
    interval: Interval,
    ctx: &ReportContext,
) -> Vec<Bucket> {
    if f.end <= f.start {
        return Vec::new();
    }
    let align = |t| bucket_start(local_date(t, ctx.tz), interval, ctx.week_start);
    let (first, last) = (align(f.start), align(f.end - 1));
    let mut sums: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut d = first;
    while d <= last {
        sums.insert(d, 0);
        d = next_bucket(d, interval);
    }
    for it in items {
        if let Some(s) = sums.get_mut(&align(it.entry.start)) {
            *s += it.seconds;
        }
    }
    sums.into_iter()
        .map(|(start, seconds)| Bucket { start, seconds })
        .collect()
}

/// Aggregates `entries` into totals, up to two grouping levels and time buckets.
pub fn aggregate(
    entries: &[TimeEntry],
    filter: &ReportFilter,
    g1: GroupBy,
    g2: Option<GroupBy>,
    interval: Interval,
    ctx: &ReportContext,
) -> ReportResult {
    let items: Vec<Item> = select_entries(entries, filter, ctx)
        .into_iter()
        .map(|e| {
            let end = e.end.unwrap_or(ctx.now);
            let seconds = match filter.rounding {
                Some((mode, step)) => rounded_duration_s(e.start, end, mode, step),
                None => (end - e.start).max(0) / 1000,
            };
            let rate = effective_rate(
                e.billable,
                e.project_id
                    .and_then(|p| ctx.projects.get(&p))
                    .and_then(|p| p.billable_rate),
                ctx.default_rate,
            );
            Item {
                entry: e,
                seconds,
                cost: cost_cents(seconds, rate),
            }
        })
        .collect();
    let refs: Vec<&Item> = items.iter().collect();
    ReportResult {
        total_seconds: items.iter().map(|i| i.seconds).sum(),
        total_cost_cents: items.iter().map(|i| i.cost).sum(),
        groups: group(&refs, g1, g2, ctx),
        buckets: buckets(&items, filter, interval, ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_id;
    use chrono::TimeZone;
    use chrono_tz::Europe::Moscow;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;

    fn at(y: i32, m: u32, d: u32, h: u32) -> Millis {
        let naive = NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap();
        Moscow
            .from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    struct Fx {
        c1: Id,
        pa: Id,
        pb: Id,
        ta: Id,
        t1: Id,
        t2: Id,
        projects: HashMap<Id, Project>,
        tasks: HashMap<Id, Task>,
    }

    fn fx() -> Fx {
        let (c1, pa, pb, ta, t1, t2) = (new_id(), new_id(), new_id(), new_id(), new_id(), new_id());
        let project = |id, rate, client| Project {
            id,
            name: String::new(),
            color: "#000000".into(),
            client_id: client,
            is_billable: true,
            billable_rate: rate,
            estimated_time: None,
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let projects = HashMap::from([
            (pa, project(pa, Some(5000), Some(c1))),
            (pb, project(pb, None, None)),
        ]);
        let tasks = HashMap::from([(
            ta,
            Task {
                id: ta,
                project_id: pa,
                name: String::new(),
                estimated_time: None,
                done_at: None,
                created_at: 0,
                updated_at: 0,
            },
        )]);
        Fx {
            c1,
            pa,
            pb,
            ta,
            t1,
            t2,
            projects,
            tasks,
        }
    }

    impl Fx {
        fn ctx(&self) -> ReportContext<'_> {
            ReportContext {
                projects: &self.projects,
                tasks: &self.tasks,
                default_rate: Some(3000),
                tz: Moscow,
                week_start: Weekday::Mon,
                now: at(2026, 11, 9, 12),
            }
        }
    }

    fn entry(start: Millis, end: Option<Millis>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: String::new(),
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

    /// 1 h entry on 2026-11-09 (Monday) starting at `h`:00.
    fn hour(h: u32) -> TimeEntry {
        entry(at(2026, 11, 9, h), Some(at(2026, 11, 9, h + 1)))
    }

    fn filter() -> ReportFilter {
        ReportFilter {
            start: at(2026, 11, 9, 0),
            end: at(2026, 11, 10, 0),
            project_ids: vec![],
            task_ids: vec![],
            client_ids: vec![],
            tag_ids: vec![],
            tag_match: TagMatch::Any,
            billable: None,
            rounding: None,
        }
    }

    fn run(f: &Fx, es: &[TimeEntry], flt: &ReportFilter, g1: GroupBy) -> ReportResult {
        aggregate(es, flt, g1, None, Interval::Day, &f.ctx())
    }

    #[test]
    fn day_groups_are_chronological_other_groups_by_time() {
        let f = fx();
        let mut flt = filter();
        flt.end = at(2026, 11, 12, 0);
        // The later day has more time.
        let short = hour(9);
        let long = entry(at(2026, 11, 10, 9), Some(at(2026, 11, 10, 12)));
        let r = run(&f, &[long, short], &flt, GroupBy::Day);
        let days: Vec<_> = r.groups.iter().map(|g| g.key.clone()).collect();
        assert_eq!(
            days,
            [
                GroupKey::Date(date(2026, 11, 9)),
                GroupKey::Date(date(2026, 11, 10))
            ]
        );
    }

    #[test]
    fn select_entries_applies_the_filter_and_keeps_input_order() {
        let f = fx();
        let mut a = hour(9);
        a.project_id = Some(f.pa);
        let mut b = hour(10);
        b.project_id = Some(f.pb);
        let mut c = hour(11);
        c.project_id = Some(f.pa);
        let outside = entry(at(2026, 11, 10, 9), Some(at(2026, 11, 10, 10)));
        let es = [a.clone(), b, outside, c.clone()];
        let mut flt = filter();
        flt.project_ids = vec![f.pa];
        let got = select_entries(&es, &flt, &f.ctx());
        assert_eq!(got, [&a, &c]);
        assert_eq!(select_entries(&es, &filter(), &f.ctx()).len(), 3);
    }

    #[test]
    fn filters_by_period_project_tag_billable() {
        let f = fx();
        let mut in_a = hour(9);
        in_a.project_id = Some(f.pa);
        in_a.task_id = Some(f.ta);
        in_a.billable = true;
        in_a.tag_ids = vec![f.t1];
        let mut in_b = hour(10);
        in_b.project_id = Some(f.pb);
        in_b.tag_ids = vec![f.t1, f.t2];
        let after = entry(at(2026, 11, 10, 9), Some(at(2026, 11, 10, 10)));
        let before = entry(at(2026, 11, 8, 9), Some(at(2026, 11, 8, 10)));
        let es = [in_a, in_b, after, before];
        let total = |flt: &ReportFilter| run(&f, &es, flt, GroupBy::Day).total_seconds;

        assert_eq!(total(&filter()), 7200); // period drops the other two
        let mut flt = filter();
        flt.project_ids = vec![f.pa];
        assert_eq!(total(&flt), 3600);
        let mut flt = filter();
        flt.task_ids = vec![f.ta];
        assert_eq!(total(&flt), 3600);
        let mut flt = filter();
        flt.client_ids = vec![f.c1];
        assert_eq!(total(&flt), 3600);
        let mut flt = filter();
        flt.billable = Some(false);
        assert_eq!(total(&flt), 3600);
        let mut flt = filter();
        flt.tag_ids = vec![f.t1];
        assert_eq!(total(&flt), 7200);
        flt.tag_ids = vec![f.t1, f.t2];
        assert_eq!(total(&flt), 7200); // Any
        flt.tag_match = TagMatch::All;
        assert_eq!(total(&flt), 3600); // only the entry with both tags
        // an entry without a project never matches a non-empty project list
        let mut flt = filter();
        flt.project_ids = vec![f.pb];
        assert_eq!(run(&f, &[hour(9)], &flt, GroupBy::Day).total_seconds, 0);
    }

    #[test]
    fn group_project_then_task() {
        let f = fx();
        let mut a1 = hour(9);
        a1.project_id = Some(f.pa);
        a1.task_id = Some(f.ta);
        let mut a2 = entry(at(2026, 11, 9, 10), Some(at(2026, 11, 9, 10) + 30 * MIN));
        a2.project_id = Some(f.pa);
        let none = hour(12);
        let r = aggregate(
            &[a1, a2, none],
            &filter(),
            GroupBy::Project,
            Some(GroupBy::Task),
            Interval::Day,
            &f.ctx(),
        );
        assert_eq!(r.total_seconds, 3600 + 1800 + 3600);
        assert_eq!(r.groups.len(), 2);
        let a = &r.groups[0];
        assert_eq!((a.key.clone(), a.seconds), (GroupKey::Id(f.pa), 5400));
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.children[0].key, GroupKey::Id(f.ta));
        assert_eq!(a.children[0].seconds, 3600);
        assert_eq!(a.children[1].key, GroupKey::None);
        assert_eq!(a.children[1].seconds, 1800);
        assert_eq!(r.groups[1].key, GroupKey::None);
        assert_eq!(r.groups[1].seconds, 3600);
        // no second level -> no children
        let r = run(&f, &[hour(9)], &filter(), GroupBy::Project);
        assert!(r.groups[0].children.is_empty());
    }

    #[test]
    fn group_by_tag_does_not_double_total() {
        let f = fx();
        let mut e = hour(9);
        e.tag_ids = vec![f.t1, f.t2];
        let r = run(&f, &[e, hour(11)], &filter(), GroupBy::Tag);
        assert_eq!(r.total_seconds, 7200);
        let secs = |k: GroupKey| r.groups.iter().find(|g| g.key == k).unwrap().seconds;
        assert_eq!(secs(GroupKey::Id(f.t1)), 3600);
        assert_eq!(secs(GroupKey::Id(f.t2)), 3600);
        assert_eq!(secs(GroupKey::None), 3600);
    }

    #[test]
    fn cost_uses_project_then_default_rate() {
        let f = fx();
        let mut a = hour(9);
        a.project_id = Some(f.pa);
        a.billable = true;
        let mut b = hour(10);
        b.project_id = Some(f.pb);
        b.billable = true;
        let mut nb = hour(11);
        nb.project_id = Some(f.pa);
        let r = run(&f, &[a, b, nb], &filter(), GroupBy::Project);
        assert_eq!(r.total_cost_cents, 8000);
        let cost = |k: Id| {
            r.groups
                .iter()
                .find(|g| g.key == GroupKey::Id(k))
                .unwrap()
                .cost_cents
        };
        assert_eq!(cost(f.pa), 5000);
        assert_eq!(cost(f.pb), 3000);
    }

    #[test]
    fn group_by_client_billable_description_day() {
        let f = fx();
        let mut a = hour(9);
        a.project_id = Some(f.pa);
        a.billable = true;
        a.description = "  work ".into();
        let mut b = hour(10);
        b.project_id = Some(f.pb);
        let both = [a, b];
        let r = run(&f, &both, &filter(), GroupBy::Client);
        let keys: Vec<_> = r.groups.iter().map(|g| g.key.clone()).collect();
        assert_eq!(keys, vec![GroupKey::Id(f.c1), GroupKey::None]);
        let r = run(&f, &both, &filter(), GroupBy::Billable);
        assert_eq!(r.groups.len(), 2);
        let r = run(&f, &both, &filter(), GroupBy::Description);
        assert_eq!(r.groups[0].key, GroupKey::Text("work".into()));
        assert_eq!(r.groups[1].key, GroupKey::None);
        let r = run(&f, &both, &filter(), GroupBy::Day);
        assert_eq!(r.groups[0].key, GroupKey::Date(date(2026, 11, 9)));
        assert_eq!(r.groups[0].seconds, 7200);
    }

    #[test]
    fn buckets_fill_gaps_week_interval() {
        let f = fx();
        let mut flt = filter();
        flt.start = at(2026, 11, 9, 0); // Monday
        flt.end = at(2026, 11, 30, 0); // 3 weeks
        let r = aggregate(
            &[hour(9)],
            &flt,
            GroupBy::Day,
            None,
            Interval::Week,
            &f.ctx(),
        );
        let got: Vec<_> = r.buckets.iter().map(|b| (b.start, b.seconds)).collect();
        assert_eq!(
            got,
            vec![
                (date(2026, 11, 9), 3600),
                (date(2026, 11, 16), 0),
                (date(2026, 11, 23), 0)
            ]
        );
        // a mid-week start aligns back to Monday
        flt.start = at(2026, 11, 11, 0);
        let r = aggregate(&[], &flt, GroupBy::Day, None, Interval::Week, &f.ctx());
        assert_eq!(r.buckets[0].start, date(2026, 11, 9));
        assert_eq!(r.buckets.len(), 3);
    }

    #[test]
    fn buckets_day_and_month() {
        let f = fx();
        let mut flt = filter();
        flt.start = at(2026, 10, 30, 0);
        flt.end = at(2026, 12, 2, 0);
        let r = aggregate(&[], &flt, GroupBy::Day, None, Interval::Month, &f.ctx());
        let starts: Vec<_> = r.buckets.iter().map(|b| b.start).collect();
        assert_eq!(
            starts,
            vec![date(2026, 10, 1), date(2026, 11, 1), date(2026, 12, 1)]
        );
        let r = aggregate(&[], &filter(), GroupBy::Day, None, Interval::Day, &f.ctx());
        assert_eq!(r.buckets.len(), 1);
    }

    #[test]
    fn running_entry_counts_until_now() {
        let f = fx();
        let running = entry(at(2026, 11, 9, 10), None); // now = 12:00
        let r = run(&f, &[running], &filter(), GroupBy::Day);
        assert_eq!(r.total_seconds, 7200);
    }

    #[test]
    fn rounding_applied_per_entry() {
        let f = fx();
        let s = at(2026, 11, 9, 9);
        let es = [
            entry(s, Some(s + 7 * MIN)),
            entry(s + HOUR, Some(s + HOUR + 7 * MIN)),
        ];
        let mut flt = filter();
        flt.rounding = Some((RoundingMode::Up, 15));
        assert_eq!(run(&f, &es, &flt, GroupBy::Day).total_seconds, 1800);
        assert_eq!(run(&f, &es, &filter(), GroupBy::Day).total_seconds, 840);
    }

    #[test]
    fn rounding_applies_before_cost() {
        let f = fx();
        let s = at(2026, 11, 9, 9);
        let mut e = entry(s, Some(s + 7 * MIN));
        e.billable = true;
        e.project_id = Some(f.pa);
        let mut flt = filter();
        flt.rounding = Some((RoundingMode::Up, 60));
        assert_eq!(run(&f, &[e], &flt, GroupBy::Day).total_cost_cents, 5000);
    }

    #[test]
    fn groups_sorted_by_seconds_desc_none_last() {
        let f = fx();
        let mut small = hour(9);
        small.project_id = Some(f.pa);
        let mut big = entry(at(2026, 11, 9, 10), Some(at(2026, 11, 9, 13)));
        big.project_id = Some(f.pb);
        let none = entry(at(2026, 11, 9, 14), Some(at(2026, 11, 9, 18))); // 4 h, largest
        let r = run(&f, &[small, big, none], &filter(), GroupBy::Project);
        let keys: Vec<_> = r.groups.iter().map(|g| g.key.clone()).collect();
        assert_eq!(
            keys,
            vec![GroupKey::Id(f.pb), GroupKey::Id(f.pa), GroupKey::None]
        );
    }

    #[test]
    fn equal_seconds_tie_broken_by_key() {
        let f = fx();
        let mut a = hour(9);
        a.project_id = Some(f.pa);
        let mut b = hour(10);
        b.project_id = Some(f.pb);
        let r = run(&f, &[a, b], &filter(), GroupBy::Project);
        assert_eq!(r.groups[0].key, GroupKey::Id(f.pa.min(f.pb)));
        assert_eq!(r.groups[1].key, GroupKey::Id(f.pa.max(f.pb)));
    }

    #[test]
    fn same_group_by_at_both_levels_mirrors() {
        let f = fx();
        let mut a = hour(9);
        a.project_id = Some(f.pa);
        let r = aggregate(
            &[a],
            &filter(),
            GroupBy::Project,
            Some(GroupBy::Project),
            Interval::Day,
            &f.ctx(),
        );
        assert_eq!(r.groups[0].children.len(), 1);
        assert_eq!(r.groups[0].children[0].seconds, 3600);
    }

    #[test]
    fn filter_round_trips_as_json() {
        let mut flt = filter();
        flt.tag_match = TagMatch::All;
        flt.rounding = Some((RoundingMode::Nearest, 15));
        let json = serde_json::to_string(&flt).unwrap();
        assert!(json.contains("\"all\"") && json.contains("\"nearest\""));
        assert_eq!(serde_json::from_str::<ReportFilter>(&json).unwrap(), flt);
        assert_eq!("week".parse::<Interval>(), Ok(Interval::Week));
        assert!("x".parse::<GroupBy>().is_err());
    }
}
