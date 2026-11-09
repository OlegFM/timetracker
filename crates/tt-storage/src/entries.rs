use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};
use tt_domain::{EntryTemplate, Id, Millis, SessionState, TimeEntry, TimerSession};

use crate::cols::{id as col_id, opt_id, opt_text, text};
use crate::{Result, StorageError};

const ENTRY_COLS: &str = "id, session_id, description, start, \"end\", billable, project_id, \
                          task_id, created_at, updated_at";

/// Intersection with `[?1, ?2)`: touching intervals do not overlap.
const OVERLAP: &str = "start < ?2 AND (\"end\" IS NULL OR \"end\" > ?1)";

fn entry_row(r: &Row) -> rusqlite::Result<TimeEntry> {
    Ok(TimeEntry {
        id: col_id(r, 0)?,
        session_id: opt_id(r, 1)?,
        description: r.get(2)?,
        start: r.get(3)?,
        end: r.get(4)?,
        billable: r.get(5)?,
        project_id: opt_id(r, 6)?,
        task_id: opt_id(r, 7)?,
        tag_ids: Vec::new(),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

/// Runs `f` inside a savepoint (nests safely in `Db::transaction`): all or nothing.
fn atomically<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT repo_op")?;
    match f() {
        Ok(v) => {
            conn.execute_batch("RELEASE repo_op")?;
            Ok(v)
        }
        Err(e) => {
            conn.execute_batch("ROLLBACK TO repo_op; RELEASE repo_op")?;
            Err(e)
        }
    }
}

fn load_tags(conn: &Connection, entry_id: Id) -> Result<Vec<Id>> {
    let mut st = conn
        .prepare("SELECT tag_id FROM time_entry_tags WHERE time_entry_id = ?1 ORDER BY tag_id")?;
    let rows = st.query_map([text(entry_id)], |r| col_id(r, 0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn write_tags(conn: &Connection, e: &TimeEntry) -> Result<()> {
    conn.execute(
        "DELETE FROM time_entry_tags WHERE time_entry_id = ?1",
        [text(e.id)],
    )?;
    // Ids of tags deleted meanwhile (a stale template or draft) are skipped.
    let mut st = conn.prepare(
        "INSERT OR IGNORE INTO time_entry_tags(time_entry_id, tag_id) SELECT ?1, id FROM tags WHERE id = ?2",
    )?;
    for tag in &e.tag_ids {
        st.execute([text(e.id), text(*tag)])?;
    }
    Ok(())
}

fn query_entries(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<TimeEntry>> {
    let mut st = conn.prepare(sql)?;
    let rows = st.query_map(params, entry_row)?;
    let mut entries = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for e in &mut entries {
        e.tag_ids = load_tags(conn, e.id)?;
    }
    Ok(entries)
}

pub fn insert_entry(conn: &Connection, e: &TimeEntry) -> Result<()> {
    atomically(conn, || {
        conn.execute(
            &format!(
                "INSERT INTO time_entries({ENTRY_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"
            ),
            params![
                text(e.id),
                opt_text(e.session_id),
                e.description,
                e.start,
                e.end,
                e.billable,
                opt_text(e.project_id),
                opt_text(e.task_id),
                e.created_at,
                e.updated_at
            ],
        )?;
        write_tags(conn, e)
    })
}

/// Overwrites all fields and the tag set.
pub fn update_entry(conn: &Connection, e: &TimeEntry) -> Result<bool> {
    atomically(conn, || {
        let n = conn.execute(
        "UPDATE time_entries SET session_id=?2, description=?3, start=?4, \"end\"=?5, billable=?6,
         project_id=?7, task_id=?8, created_at=?9, updated_at=?10 WHERE id=?1",
        params![
            text(e.id),
            opt_text(e.session_id),
            e.description,
            e.start,
            e.end,
            e.billable,
            opt_text(e.project_id),
            opt_text(e.task_id),
            e.created_at,
            e.updated_at
        ],
    )?;
        if n == 0 {
            return Ok(false);
        }
        write_tags(conn, e)?;
        Ok(true)
    })
}

pub fn delete_entry(conn: &Connection, id: Id) -> Result<bool> {
    let n = conn.execute("DELETE FROM time_entries WHERE id = ?1", [text(id)])?;
    Ok(n > 0)
}

pub fn get_entry(conn: &Connection, id: Id) -> Result<Option<TimeEntry>> {
    let mut v = query_entries(
        conn,
        &format!("SELECT {ENTRY_COLS} FROM time_entries WHERE id = ?1"),
        [text(id)],
    )?;
    Ok(v.pop())
}

pub fn running_entry(conn: &Connection) -> Result<Option<TimeEntry>> {
    let mut v = query_entries(
        conn,
        &format!(
            "SELECT {ENTRY_COLS} FROM time_entries WHERE \"end\" IS NULL ORDER BY start DESC LIMIT 1"
        ),
        [],
    )?;
    Ok(v.pop())
}

/// Entries intersecting `[start, end)` (running ones included), by start ascending.
pub fn entries_in_range(conn: &Connection, start: Millis, end: Millis) -> Result<Vec<TimeEntry>> {
    query_entries(
        conn,
        &format!("SELECT {ENTRY_COLS} FROM time_entries WHERE {OVERLAP} ORDER BY start, id"),
        params![start, end],
    )
}

pub fn entries_of_session(conn: &Connection, session_id: Id) -> Result<Vec<TimeEntry>> {
    query_entries(
        conn,
        &format!("SELECT {ENTRY_COLS} FROM time_entries WHERE session_id = ?1 ORDER BY start, id"),
        [text(session_id)],
    )
}

/// Latest entries first.
pub fn recent_entries(conn: &Connection, limit: u32) -> Result<Vec<TimeEntry>> {
    query_entries(
        conn,
        &format!("SELECT {ENTRY_COLS} FROM time_entries ORDER BY start DESC, id DESC LIMIT ?1"),
        [limit],
    )
}

/// Entries that started before `before`, latest first, at most `limit`.
pub fn entries_started_before(
    conn: &Connection,
    before: Millis,
    limit: u32,
) -> Result<Vec<TimeEntry>> {
    query_entries(
        conn,
        &format!(
            "SELECT {ENTRY_COLS} FROM time_entries WHERE start < ?1              ORDER BY start DESC, id DESC LIMIT ?2"
        ),
        params![before, limit],
    )
}

/// Whether any entry other than `exclude` intersects `[start, end)`.
pub fn overlapping(
    conn: &Connection,
    start: Millis,
    end: Millis,
    exclude: Option<Id>,
) -> Result<bool> {
    Ok(conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM time_entries WHERE {OVERLAP} AND (?3 IS NULL OR id != ?3))"
        ),
        params![start, end, opt_text(exclude)],
        |r| r.get(0),
    )?)
}

fn spent_ms(conn: &Connection, column: &str, id: Id, now: Millis) -> Result<i64> {
    Ok(conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(COALESCE(\"end\", ?2) - start), 0) FROM time_entries WHERE {column} = ?1"
        ),
        params![text(id), now],
        |r| r.get(0),
    )?)
}

pub fn spent_ms_for_project(conn: &Connection, id: Id, now: Millis) -> Result<i64> {
    spent_ms(conn, "project_id", id, now)
}

pub fn spent_ms_for_task(conn: &Connection, id: Id, now: Millis) -> Result<i64> {
    spent_ms(conn, "task_id", id, now)
}

/// Tracked ms per id of `column` (grouped; a running entry counts up to `now`).
fn spent_grouped(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<HashMap<Id, i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |r| Ok((col_id(r, 0)?, r.get::<_, i64>(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Tracked ms of every project that has entries.
pub fn spent_ms_by_project(conn: &Connection, now: Millis) -> Result<HashMap<Id, i64>> {
    spent_grouped(
        conn,
        "SELECT project_id, SUM(COALESCE(\"end\", ?1) - start) FROM time_entries          WHERE project_id IS NOT NULL GROUP BY project_id",
        params![now],
    )
}

/// Tracked ms of every task of `project_id` that has entries.
pub fn spent_ms_by_task(
    conn: &Connection,
    project_id: Id,
    now: Millis,
) -> Result<HashMap<Id, i64>> {
    spent_grouped(
        conn,
        "SELECT task_id, SUM(COALESCE(\"end\", ?2) - start) FROM time_entries          WHERE project_id = ?1 AND task_id IS NOT NULL GROUP BY task_id",
        params![text(project_id), now],
    )
}

/// Number of time entries per tag (tags without entries are absent).
pub fn entry_counts_by_tag(conn: &Connection) -> Result<HashMap<Id, i64>> {
    spent_grouped(
        conn,
        "SELECT tag_id, COUNT(*) FROM time_entry_tags GROUP BY tag_id",
        [],
    )
}

// ---- sessions ----

fn state_text(s: SessionState) -> &'static str {
    match s {
        SessionState::Running => "running",
        SessionState::Paused => "paused",
        SessionState::Stopped => "stopped",
    }
}

fn parse_state(s: &str) -> Result<SessionState> {
    match s {
        "running" => Ok(SessionState::Running),
        "paused" => Ok(SessionState::Paused),
        "stopped" => Ok(SessionState::Stopped),
        other => Err(StorageError::Corrupt(format!(
            "bad session state {other:?}"
        ))),
    }
}

pub fn insert_session(conn: &Connection, s: &TimerSession) -> Result<()> {
    conn.execute(
        "INSERT INTO timer_sessions(id, started_at, ended_at, state) VALUES (?1,?2,?3,?4)",
        params![text(s.id), s.started_at, s.ended_at, state_text(s.state)],
    )?;
    Ok(())
}

/// Does not touch the stored template.
pub fn update_session(conn: &Connection, s: &TimerSession) -> Result<bool> {
    let n = conn.execute(
        "UPDATE timer_sessions SET started_at=?2, ended_at=?3, state=?4 WHERE id=?1",
        params![text(s.id), s.started_at, s.ended_at, state_text(s.state)],
    )?;
    Ok(n > 0)
}

/// The session that is not stopped (latest `started_at` if there are several).
pub fn active_session(conn: &Connection) -> Result<Option<TimerSession>> {
    let found = conn
        .query_row(
            "SELECT id, started_at, ended_at, state FROM timer_sessions
             WHERE state != 'stopped' ORDER BY started_at DESC LIMIT 1",
            [],
            |r| {
                Ok((
                    col_id(r, 0)?,
                    r.get::<_, Millis>(1)?,
                    r.get::<_, Option<Millis>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    found
        .map(|(id, started_at, ended_at, state)| {
            Ok(TimerSession {
                id,
                started_at,
                ended_at,
                state: parse_state(&state)?,
            })
        })
        .transpose()
}

pub fn set_session_template(conn: &Connection, id: Id, t: &EntryTemplate) -> Result<()> {
    conn.execute(
        "UPDATE timer_sessions SET template_json = ?2 WHERE id = ?1",
        params![text(id), serde_json::to_string(t)?],
    )?;
    Ok(())
}

/// Templates of the sessions that are not stopped.
pub fn open_session_templates(conn: &Connection) -> Result<Vec<(Id, EntryTemplate)>> {
    let mut st = conn.prepare(
        "SELECT id, template_json FROM timer_sessions
         WHERE state != 'stopped' AND template_json IS NOT NULL",
    )?;
    let rows = st.query_map([], |r| Ok((col_id(r, 0)?, r.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        Ok((id, serde_json::from_str(&json)?))
    })
    .collect()
}

pub fn session_template(conn: &Connection, id: Id) -> Result<Option<EntryTemplate>> {
    let json: Option<Option<String>> = conn
        .query_row(
            "SELECT template_json FROM timer_sessions WHERE id = ?1",
            [text(id)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(json
        .flatten()
        .map(|j| serde_json::from_str(&j))
        .transpose()?)
}
