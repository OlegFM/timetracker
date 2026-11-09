//! Timer sessions: start / pause / resume / stop and splitting at local midnights.

use rusqlite::Connection;
use tt_domain::{
    EntryTemplate, Id, Millis, SessionState, TimeEntry, TimerSession, Tz, day_total, local_date,
    new_id, session_total, split_at_local_midnights, start_of_local_day,
};
use tt_storage::{Db, entries};

use crate::clock::Clock;
use crate::error::{Result, ServiceError};
use crate::tx::in_tx;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimerStatus {
    pub session: Option<TimerSession>,
    /// The running entry, else the latest entry of the session.
    pub current: Option<TimeEntry>,
    pub template: Option<EntryTemplate>,
    pub session_ms: i64,
    pub today_ms: i64,
}

pub struct TimerService<C: Clock> {
    clock: C,
}

impl<C: Clock> TimerService<C> {
    pub fn new(clock: C) -> Self {
        Self { clock }
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }

    pub fn status(&self, db: &Db, tz: Tz) -> Result<TimerStatus> {
        Ok(status_of(db.conn(), self.clock.now(), tz)?)
    }

    /// Stops the active session (if any) and starts a new running one.
    pub fn start(&self, db: &mut Db, t: EntryTemplate, tz: Tz) -> Result<TimerStatus> {
        let now = self.clock.now();
        in_tx(db, |c| {
            if let Some(s) = entries::active_session(c)? {
                stop_session(c, s, now, tz)?;
            }
            // A running entry outside any active session would block the new one.
            close_running(c, now, tz)?;
            let s = TimerSession {
                id: new_id(),
                started_at: now,
                ended_at: None,
                state: SessionState::Running,
            };
            entries::insert_session(c, &s)?;
            entries::set_session_template(c, s.id, &t)?;
            entries::insert_entry(c, &new_entry(s.id, &t, now))?;
            Ok(status_of(c, now, tz)?)
        })
    }

    pub fn pause(&self, db: &mut Db, tz: Tz) -> Result<TimerStatus> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut s = active_in(c, SessionState::Running, "not running")?;
            close_running(c, now, tz)?;
            s.state = SessionState::Paused;
            entries::update_session(c, &s)?;
            Ok(status_of(c, now, tz)?)
        })
    }

    /// New running entry from the session template (fallback: the last entry).
    pub fn resume(&self, db: &mut Db, tz: Tz) -> Result<TimerStatus> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut s = active_in(c, SessionState::Paused, "not paused")?;
            let t = match entries::session_template(c, s.id)? {
                Some(t) => t,
                None => entries::entries_of_session(c, s.id)?
                    .last()
                    .map(EntryTemplate::from_entry)
                    .ok_or(ServiceError::NotFound)?,
            };
            close_running(c, now, tz)?;
            entries::insert_entry(c, &new_entry(s.id, &t, now))?;
            s.state = SessionState::Running;
            entries::update_session(c, &s)?;
            Ok(status_of(c, now, tz)?)
        })
    }

    pub fn stop(&self, db: &mut Db, tz: Tz) -> Result<TimerStatus> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let s = entries::active_session(c)?.ok_or(ServiceError::Invalid("not active"))?;
            stop_session(c, s, now, tz)?;
            Ok(status_of(c, now, tz)?)
        })
    }

    /// Stores the template and applies it to the running entry (after the midnight
    /// rollover); closed entries stay as is.
    pub fn update_template(&self, db: &mut Db, t: EntryTemplate, tz: Tz) -> Result<TimerStatus> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let s = entries::active_session(c)?.ok_or(ServiceError::Invalid("not active"))?;
            entries::set_session_template(c, s.id, &t)?;
            // The part before today's midnight keeps the old attributes.
            rollover_in(c, now, tz)?;
            let running = entries::running_entry(c)?.filter(|e| e.session_id == Some(s.id));
            if let Some(mut e) = running {
                e.description = t.description.clone();
                e.project_id = t.project_id;
                e.task_id = t.task_id;
                e.tag_ids = t.tag_ids.clone();
                e.billable = t.billable;
                e.updated_at = now;
                update_existing(c, &e)?;
            }
            Ok(status_of(c, now, tz)?)
        })
    }

    /// Splits a running entry that started before today's local midnight; the piece
    /// from today's start stays running. Returns whether anything changed.
    pub fn rollover(&self, db: &mut Db, tz: Tz) -> Result<bool> {
        let now = self.clock.now();
        in_tx(db, |c| rollover_in(c, now, tz))
    }
}

fn rollover_in(c: &Connection, now: Millis, tz: Tz) -> Result<bool> {
    let Some(e) = entries::running_entry(c)? else {
        return Ok(false);
    };
    let day_start = start_of_local_day(local_date(now, tz), tz);
    if e.start >= day_start {
        return Ok(false);
    }
    close_at(c, &e, day_start, now, tz)?;
    entries::insert_entry(c, &piece(&e, day_start, None, now))?;
    Ok(true)
}

fn status_of(c: &Connection, now: Millis, tz: Tz) -> tt_storage::Result<TimerStatus> {
    let Some(session) = entries::active_session(c)? else {
        return Ok(TimerStatus::default());
    };
    let list = entries::entries_of_session(c, session.id)?;
    let current = match entries::running_entry(c)? {
        Some(e) => Some(e),
        None => list.last().cloned(),
    };
    Ok(TimerStatus {
        template: entries::session_template(c, session.id)?,
        session_ms: session_total(&list, now),
        today_ms: day_total(&list, local_date(now, tz), now, tz),
        session: Some(session),
        current,
    })
}

/// The active session if it is in `state`, else `Invalid(msg)`.
fn active_in(c: &Connection, state: SessionState, msg: &'static str) -> Result<TimerSession> {
    entries::active_session(c)?
        .filter(|s| s.state == state)
        .ok_or(ServiceError::Invalid(msg))
}

fn stop_session(c: &Connection, mut s: TimerSession, now: Millis, tz: Tz) -> Result<()> {
    close_running(c, now, tz)?;
    s.state = SessionState::Stopped;
    s.ended_at = Some(now);
    entries::update_session(c, &s)?;
    Ok(())
}

/// Pauses a running session at `now` (as [`TimerService::pause`]); a paused or stopped
/// timer is left as it is. Used on the copy written by a backup.
pub(crate) fn pause_running_in(c: &Connection, now: Millis, tz: Tz) -> Result<()> {
    close_running(c, now, tz)?;
    if let Some(mut s) = entries::active_session(c)?.filter(|s| s.state == SessionState::Running) {
        s.state = SessionState::Paused;
        entries::update_session(c, &s)?;
    }
    Ok(())
}

fn close_running(c: &Connection, now: Millis, tz: Tz) -> Result<()> {
    if let Some(e) = entries::running_entry(c)? {
        close_at(c, &e, now, now, tz)?;
    }
    Ok(())
}

/// Closes `e` at `end`, split at local midnights: `e` keeps the first piece, every
/// further piece becomes a new entry with the same attributes. An `end` not after the
/// start (stopped at once, or the clock was moved back) would leave an empty entry: `e`
/// is deleted instead.
fn close_at(c: &Connection, e: &TimeEntry, end: Millis, now: Millis, tz: Tz) -> Result<()> {
    if end <= e.start {
        entries::delete_entry(c, e.id)?;
        return Ok(());
    }
    let pieces = split_at_local_midnights(e.start, end, tz);
    let first_end = pieces.first().map_or(end, |p| p.1);
    update_existing(
        c,
        &TimeEntry {
            end: Some(first_end),
            updated_at: now,
            ..e.clone()
        },
    )?;
    for &(start, piece_end) in pieces.iter().skip(1) {
        entries::insert_entry(c, &piece(e, start, Some(piece_end), now))?;
    }
    Ok(())
}

/// A new entry copying `e`'s session and attributes.
fn piece(e: &TimeEntry, start: Millis, end: Option<Millis>, now: Millis) -> TimeEntry {
    TimeEntry {
        id: new_id(),
        start,
        end,
        created_at: now,
        updated_at: now,
        ..e.clone()
    }
}

fn new_entry(session_id: Id, t: &EntryTemplate, now: Millis) -> TimeEntry {
    TimeEntry {
        id: new_id(),
        session_id: Some(session_id),
        description: t.description.clone(),
        start: now,
        end: None,
        billable: t.billable,
        project_id: t.project_id,
        task_id: t.task_id,
        tag_ids: t.tag_ids.clone(),
        created_at: now,
        updated_at: now,
    }
}

fn update_existing(c: &Connection, e: &TimeEntry) -> Result<()> {
    if entries::update_entry(c, e)? {
        Ok(())
    } else {
        Err(ServiceError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use chrono::{NaiveDate, TimeZone};
    use tt_domain::{Project, Tag};
    use tt_storage::catalog;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const MSK: Tz = Tz::Europe__Moscow;
    const TOKYO: Tz = Tz::Asia__Tokyo;

    /// Instant of a local wall-clock time in `tz`.
    fn at_tz(tz: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Millis {
        let naive = NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap();
        tz.from_local_datetime(&naive)
            .earliest()
            .unwrap()
            .timestamp_millis()
    }

    /// Moscow wall-clock time in November 2026.
    fn at(d: u32, h: u32, mi: u32) -> Millis {
        at_tz(MSK, 2026, 11, d, h, mi)
    }

    fn tpl(description: &str) -> EntryTemplate {
        EntryTemplate {
            description: description.into(),
            project_id: None,
            task_id: None,
            tag_ids: vec![],
            billable: false,
        }
    }

    fn setup(now: Millis) -> (TimerService<FixedClock>, Db) {
        (
            TimerService::new(FixedClock::new(now)),
            Db::open_in_memory().unwrap(),
        )
    }

    fn session_entries(db: &Db, session_id: Id) -> Vec<TimeEntry> {
        entries::entries_of_session(db.conn(), session_id).unwrap()
    }

    fn spans(list: &[TimeEntry]) -> Vec<(Millis, Option<Millis>)> {
        list.iter().map(|e| (e.start, e.end)).collect()
    }

    fn all_entries(db: &Db) -> Vec<TimeEntry> {
        entries::entries_in_range(db.conn(), i64::MIN, i64::MAX).unwrap()
    }

    fn session_row(db: &Db, id: Id) -> (String, Option<Millis>) {
        db.conn()
            .query_row(
                "SELECT state, ended_at FROM timer_sessions WHERE id = ?1",
                [id.hyphenated().to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn session_id(st: &TimerStatus) -> Id {
        st.session.as_ref().unwrap().id
    }

    #[test]
    fn start_creates_running_session_and_entry() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let st = svc.start(&mut db, tpl("a"), MSK).unwrap();

        let s = st.session.clone().unwrap();
        assert_eq!(s.state, SessionState::Running);
        assert_eq!(s.started_at, at(9, 10, 0));
        assert_eq!(s.ended_at, None);
        let cur = st.current.clone().unwrap();
        assert_eq!((cur.start, cur.end), (at(9, 10, 0), None));
        assert_eq!(cur.session_id, Some(s.id));
        assert_eq!(cur.description, "a");
        assert_eq!(
            (cur.created_at, cur.updated_at),
            (at(9, 10, 0), at(9, 10, 0))
        );
        assert_eq!(st.template, Some(tpl("a")));
        assert_eq!(st.session_ms, 0);

        svc.clock().advance(5 * MIN);
        let st = svc.status(&db, MSK).unwrap();
        assert_eq!(st.session_ms, 5 * MIN);
        assert_eq!(st.today_ms, 5 * MIN);
        assert_eq!(st.current.unwrap().id, cur.id);
    }

    #[test]
    fn status_without_session_is_empty() {
        let (svc, db) = setup(at(9, 10, 0));
        assert_eq!(svc.status(&db, MSK).unwrap(), TimerStatus::default());
    }

    #[test]
    fn pause_closes_entry_and_records_no_gap() {
        let (svc, mut db) = setup(at(9, 10, 0));
        svc.start(&mut db, tpl("a"), MSK).unwrap();
        svc.clock().set(at(9, 10, 30));
        let st = svc.pause(&mut db, MSK).unwrap();
        assert_eq!(st.session.as_ref().unwrap().state, SessionState::Paused);
        svc.clock().set(at(9, 11, 0));
        svc.resume(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 11, 15));
        let st = svc.pause(&mut db, MSK).unwrap();

        assert_eq!(
            spans(&session_entries(&db, session_id(&st))),
            vec![
                (at(9, 10, 0), Some(at(9, 10, 30))),
                (at(9, 11, 0), Some(at(9, 11, 15)))
            ]
        );
        assert_eq!(all_entries(&db).len(), 2);
        assert_eq!(st.session_ms, 45 * MIN);
        let cur = st.current.unwrap();
        assert_eq!((cur.start, cur.end), (at(9, 11, 0), Some(at(9, 11, 15))));
        assert_eq!(cur.description, "a");
    }

    #[test]
    fn stop_after_pause_ends_session() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 10, 30));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 10, 45));
        let st = svc.stop(&mut db, MSK).unwrap();

        assert_eq!(st, TimerStatus::default());
        assert_eq!(
            session_row(&db, sid),
            ("stopped".into(), Some(at(9, 10, 45)))
        );
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(at(9, 10, 0), Some(at(9, 10, 30)))]
        );
        assert!(entries::running_entry(db.conn()).unwrap().is_none());
    }

    #[test]
    fn stop_running_closes_entry() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 10, 20));
        svc.stop(&mut db, MSK).unwrap();
        assert_eq!(
            session_row(&db, sid),
            ("stopped".into(), Some(at(9, 10, 20)))
        );
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(at(9, 10, 0), Some(at(9, 10, 20)))]
        );
    }

    #[test]
    fn start_other_task_stops_current_session() {
        for pause_first in [false, true] {
            let (svc, mut db) = setup(at(9, 10, 0));
            let old = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
            if pause_first {
                svc.clock().set(at(9, 10, 30));
                svc.pause(&mut db, MSK).unwrap();
            }
            svc.clock().set(at(9, 10, 45));
            let st = svc.start(&mut db, tpl("b"), MSK).unwrap();

            let new = st.session.clone().unwrap();
            assert_ne!(new.id, old);
            assert_eq!(new.state, SessionState::Running);
            assert_eq!(
                session_row(&db, old),
                ("stopped".into(), Some(at(9, 10, 45)))
            );
            let old_end = if pause_first {
                at(9, 10, 30)
            } else {
                at(9, 10, 45)
            };
            assert_eq!(
                spans(&session_entries(&db, old)),
                vec![(at(9, 10, 0), Some(old_end))]
            );
            let cur = st.current.unwrap();
            assert_eq!((cur.start, cur.end), (at(9, 10, 45), None));
            assert_eq!(cur.description, "b");
            assert_eq!(cur.session_id, Some(new.id));
            assert_eq!(st.session_ms, 0);
        }
    }

    #[test]
    fn update_template_applies_to_running_and_future_not_past() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 10, 30));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 11, 0));
        svc.resume(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 11, 10));
        let mut b = tpl("b");
        b.billable = true;
        let st = svc.update_template(&mut db, b.clone(), MSK).unwrap();
        assert_eq!(st.template, Some(b.clone()));
        let cur = st.current.unwrap();
        assert_eq!((cur.description.as_str(), cur.billable), ("b", true));
        assert_eq!(cur.updated_at, at(9, 11, 10));
        assert_eq!(cur.end, None);

        svc.clock().set(at(9, 11, 20));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 11, 30));
        svc.resume(&mut db, MSK).unwrap();

        let list = session_entries(&db, sid);
        let descs: Vec<_> = list.iter().map(|e| e.description.as_str()).collect();
        assert_eq!(descs, ["a", "b", "b"]);
        assert!(!list[0].billable && list[1].billable && list[2].billable);
        assert_eq!(list[0].updated_at, at(9, 10, 30));
    }

    #[test]
    fn update_template_while_paused_used_on_resume() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 10, 30));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 10, 40));
        let st = svc.update_template(&mut db, tpl("b"), MSK).unwrap();
        assert_eq!(st.template, Some(tpl("b")));
        assert_eq!(st.current.unwrap().description, "a");

        svc.clock().set(at(9, 11, 0));
        let st = svc.resume(&mut db, MSK).unwrap();
        assert_eq!(st.session.unwrap().state, SessionState::Running);
        let cur = st.current.unwrap();
        assert_eq!((cur.start, cur.end), (at(9, 11, 0), None));
        assert_eq!(cur.description, "b");
        let descs: Vec<_> = session_entries(&db, sid)
            .into_iter()
            .map(|e| e.description)
            .collect();
        assert_eq!(descs, ["a", "b"]);
    }

    #[test]
    fn pause_overnight_resume_next_day() {
        let (svc, mut db) = setup(at(9, 17, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 18, 0));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(10, 9, 0));
        assert!(!svc.rollover(&mut db, MSK).unwrap());
        svc.resume(&mut db, MSK).unwrap();
        svc.clock().set(at(10, 9, 30));
        let st = svc.status(&db, MSK).unwrap();

        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(at(9, 17, 0), Some(at(9, 18, 0))), (at(10, 9, 0), None)]
        );
        assert_eq!(st.today_ms, 30 * MIN);
        assert_eq!(st.session_ms, HOUR + 30 * MIN);
    }

    #[test]
    fn rollover_at_midnight() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let st = svc.start(&mut db, tpl("a"), MSK).unwrap();
        let (sid, orig) = (session_id(&st), st.current.unwrap().id);
        svc.clock().set(at(10, 0, 10));
        assert!(svc.rollover(&mut db, MSK).unwrap());
        assert!(!svc.rollover(&mut db, MSK).unwrap());

        let list = session_entries(&db, sid);
        assert_eq!(
            spans(&list),
            vec![(at(9, 23, 30), Some(at(10, 0, 0))), (at(10, 0, 0), None)]
        );
        assert_eq!(list[0].id, orig);
        assert_eq!(list[1].description, "a");
        let st = svc.status(&db, MSK).unwrap();
        assert_eq!(st.current.unwrap().id, list[1].id);
        assert_eq!(st.today_ms, 10 * MIN);
        assert_eq!(st.session_ms, 40 * MIN);
    }

    #[test]
    fn update_template_after_midnight_without_tick_keeps_past_piece() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(10, 0, 10));
        svc.update_template(&mut db, tpl("b"), MSK).unwrap();

        let list = session_entries(&db, sid);
        assert_eq!(
            spans(&list),
            vec![(at(9, 23, 30), Some(at(10, 0, 0))), (at(10, 0, 0), None)]
        );
        assert_eq!(list[0].description, "a");
        assert_eq!(list[1].description, "b");
    }

    #[test]
    fn rollover_across_dst_gap_midnight() {
        let sp = Tz::America__Sao_Paulo;
        let start = at_tz(sp, 2018, 11, 3, 22, 0);
        let now = at_tz(sp, 2018, 11, 4, 2, 0);
        let (svc, mut db) = setup(start);
        let sid = session_id(&svc.start(&mut db, tpl("a"), sp).unwrap());
        svc.clock().set(now);
        assert!(svc.rollover(&mut db, sp).unwrap());
        // 2018-11-04 00:00 does not exist; the day starts at 01:00 local = 03:00 UTC.
        let day_start = at_tz(sp, 2018, 11, 4, 1, 0);
        assert_eq!(day_start, 1_541_300_400_000);
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(start, Some(day_start)), (day_start, None)]
        );
    }

    #[test]
    fn rollover_at_exact_midnight() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(10, 0, 0));
        assert!(svc.rollover(&mut db, MSK).unwrap());
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(at(9, 23, 30), Some(at(10, 0, 0))), (at(10, 0, 0), None)]
        );
    }

    #[test]
    fn rollover_without_running_entry_is_noop() {
        let (svc, mut db) = setup(at(9, 10, 0));
        assert!(!svc.rollover(&mut db, MSK).unwrap());
        svc.start(&mut db, tpl("a"), MSK).unwrap();
        svc.clock().set(at(9, 23, 59));
        assert!(!svc.rollover(&mut db, MSK).unwrap());
        assert_eq!(all_entries(&db).len(), 1);
    }

    #[test]
    fn rollover_after_three_days_asleep() {
        let (svc, mut db) = setup(at(9, 22, 0));
        let project = Project {
            id: new_id(),
            name: "p".into(),
            color: "#112233".into(),
            client_id: None,
            is_billable: true,
            billable_rate: None,
            estimated_time: None,
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        };
        catalog::insert_project(db.conn(), &project).unwrap();
        let mut tags: Vec<Id> = Vec::new();
        for name in ["x", "y"] {
            let tag = Tag {
                id: new_id(),
                name: name.into(),
                created_at: 0,
                updated_at: 0,
            };
            catalog::insert_tag(db.conn(), &tag).unwrap();
            tags.push(tag.id);
        }
        tags.sort();
        let t = EntryTemplate {
            description: "deep".into(),
            project_id: Some(project.id),
            task_id: None,
            tag_ids: tags.clone(),
            billable: true,
        };
        let st = svc.start(&mut db, t, MSK).unwrap();
        let (sid, orig) = (session_id(&st), st.current.unwrap().id);

        svc.clock().set(at(12, 8, 0));
        assert!(svc.rollover(&mut db, MSK).unwrap());

        let list = session_entries(&db, sid);
        assert_eq!(
            spans(&list),
            vec![
                (at(9, 22, 0), Some(at(10, 0, 0))),
                (at(10, 0, 0), Some(at(11, 0, 0))),
                (at(11, 0, 0), Some(at(12, 0, 0))),
                (at(12, 0, 0), None),
            ]
        );
        assert_eq!(list[0].id, orig);
        let mut ids: Vec<Id> = list.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4);
        for e in &list {
            assert_eq!(e.tag_ids, tags);
            assert_eq!(e.project_id, Some(project.id));
            assert_eq!(e.description, "deep");
            assert!(e.billable);
            assert_eq!(e.session_id, Some(sid));
        }
        assert_eq!(all_entries(&db).len(), 4);
        let st = svc.status(&db, MSK).unwrap();
        assert_eq!(st.session_ms, 58 * HOUR);
        assert_eq!(st.today_ms, 8 * HOUR);
    }

    #[test]
    fn rollover_uses_current_timezone() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(10, 0, 10));
        assert!(svc.rollover(&mut db, MSK).unwrap());
        let closed = session_entries(&db, sid)[0].clone();
        assert_eq!(
            (closed.start, closed.end),
            (at(9, 23, 30), Some(at(10, 0, 0)))
        );

        // The user switches to Tokyo; the next local midnight is Tokyo's.
        svc.clock().set(at_tz(TOKYO, 2026, 11, 11, 1, 0));
        assert!(svc.rollover(&mut db, TOKYO).unwrap());

        let list = session_entries(&db, sid);
        assert_eq!(list[0], closed);
        assert_eq!(
            spans(&list[1..]),
            vec![
                (at(10, 0, 0), Some(at_tz(TOKYO, 2026, 11, 11, 0, 0))),
                (at_tz(TOKYO, 2026, 11, 11, 0, 0), None),
            ]
        );
        assert!(!svc.rollover(&mut db, TOKYO).unwrap());
    }

    #[test]
    fn pause_across_midnight_without_tick_splits() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let st = svc.start(&mut db, tpl("a"), MSK).unwrap();
        let (sid, orig) = (session_id(&st), st.current.unwrap().id);
        svc.clock().set(at(10, 0, 10));
        let st = svc.pause(&mut db, MSK).unwrap();

        let list = session_entries(&db, sid);
        assert_eq!(
            spans(&list),
            vec![
                (at(9, 23, 30), Some(at(10, 0, 0))),
                (at(10, 0, 0), Some(at(10, 0, 10)))
            ]
        );
        assert_eq!(list[0].id, orig);
        assert_eq!(list[1].description, "a");
        assert_eq!(st.session_ms, 40 * MIN);
        assert_eq!(st.today_ms, 10 * MIN);
    }

    #[test]
    fn stop_across_midnight_without_tick_splits() {
        let (svc, mut db) = setup(at(9, 23, 30));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(10, 0, 10));
        svc.stop(&mut db, MSK).unwrap();
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![
                (at(9, 23, 30), Some(at(10, 0, 0))),
                (at(10, 0, 0), Some(at(10, 0, 10)))
            ]
        );
        assert_eq!(
            session_row(&db, sid),
            ("stopped".into(), Some(at(10, 0, 10)))
        );
    }

    #[test]
    fn clock_going_backwards_drops_the_empty_entry() {
        let (svc, mut db) = setup(at(9, 9, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.clock().set(at(9, 9, 30));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 10, 0));
        svc.resume(&mut db, MSK).unwrap();
        // The clock is moved back before the running entry's start.
        svc.clock().set(at(9, 9, 50));
        assert!(!svc.rollover(&mut db, MSK).unwrap());
        let st = svc.pause(&mut db, MSK).unwrap();
        assert_eq!(
            spans(&session_entries(&db, sid)),
            vec![(at(9, 9, 0), Some(at(9, 9, 30)))]
        );
        assert_eq!(st.session_ms, 30 * MIN);
        // The session goes on from its template.
        svc.clock().set(at(9, 10, 30));
        let cur = svc.resume(&mut db, MSK).unwrap().current.unwrap();
        assert_eq!(
            (cur.start, cur.end, cur.description.as_str()),
            (at(9, 10, 30), None, "a")
        );
    }

    #[test]
    fn stop_at_the_start_instant_leaves_no_entry() {
        let (svc, mut db) = setup(at(9, 10, 0));
        let sid = session_id(&svc.start(&mut db, tpl("a"), MSK).unwrap());
        svc.stop(&mut db, MSK).unwrap();
        assert!(session_entries(&db, sid).is_empty());
        assert_eq!(session_row(&db, sid).0, "stopped");
    }

    #[test]
    fn recover_running_session_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tt.db");
        {
            let mut db = Db::open(&path).unwrap();
            let svc = TimerService::new(FixedClock::new(at(9, 10, 0)));
            svc.start(&mut db, tpl("a"), MSK).unwrap();
        }
        let db = Db::open(&path).unwrap();
        let svc = TimerService::new(FixedClock::new(at(9, 10, 20)));
        let st = svc.status(&db, MSK).unwrap();
        assert_eq!(st.session.unwrap().state, SessionState::Running);
        let cur = st.current.unwrap();
        assert_eq!((cur.start, cur.end), (at(9, 10, 0), None));
        assert_eq!(st.session_ms, 20 * MIN);
        assert_eq!(st.template, Some(tpl("a")));
    }

    #[test]
    fn pause_when_not_running_is_invalid() {
        let (svc, mut db) = setup(at(9, 10, 0));
        assert!(matches!(
            svc.pause(&mut db, MSK),
            Err(ServiceError::Invalid("not running"))
        ));
        svc.start(&mut db, tpl("a"), MSK).unwrap();
        svc.clock().set(at(9, 10, 30));
        svc.pause(&mut db, MSK).unwrap();
        svc.clock().set(at(9, 10, 40));
        assert!(matches!(
            svc.pause(&mut db, MSK),
            Err(ServiceError::Invalid("not running"))
        ));
        assert_eq!(all_entries(&db).len(), 1);
    }

    #[test]
    fn other_transitions_validate_state() {
        let (svc, mut db) = setup(at(9, 10, 0));
        assert!(matches!(
            svc.resume(&mut db, MSK),
            Err(ServiceError::Invalid("not paused"))
        ));
        assert!(matches!(
            svc.stop(&mut db, MSK),
            Err(ServiceError::Invalid("not active"))
        ));
        assert!(matches!(
            svc.update_template(&mut db, tpl("b"), MSK),
            Err(ServiceError::Invalid("not active"))
        ));
        svc.start(&mut db, tpl("a"), MSK).unwrap();
        assert!(matches!(
            svc.resume(&mut db, MSK),
            Err(ServiceError::Invalid("not paused"))
        ));
        svc.stop(&mut db, MSK).unwrap();
        assert!(matches!(
            svc.stop(&mut db, MSK),
            Err(ServiceError::Invalid("not active"))
        ));
    }
}
