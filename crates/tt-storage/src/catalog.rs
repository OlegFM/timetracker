use rusqlite::{Connection, OptionalExtension, Row, params};
use tt_domain::{Client, Id, Project, Tag, Task};

use crate::Result;
use crate::cols::{id as col_id, opt_id, opt_text, text};

// ---- clients ----

const CLIENT_COLS: &str = "id, name, archived_at, created_at, updated_at";

fn client_row(r: &Row) -> rusqlite::Result<Client> {
    Ok(Client {
        id: col_id(r, 0)?,
        name: r.get(1)?,
        archived_at: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
    })
}

pub fn insert_client(conn: &Connection, c: &Client) -> Result<()> {
    conn.execute(
        "INSERT INTO clients(id, name, archived_at, created_at, updated_at) VALUES (?1,?2,?3,?4,?5)",
        params![text(c.id), c.name, c.archived_at, c.created_at, c.updated_at],
    )?;
    Ok(())
}

pub fn update_client(conn: &Connection, c: &Client) -> Result<bool> {
    let n = conn.execute(
        "UPDATE clients SET name=?2, archived_at=?3, created_at=?4, updated_at=?5 WHERE id=?1",
        params![
            text(c.id),
            c.name,
            c.archived_at,
            c.created_at,
            c.updated_at
        ],
    )?;
    Ok(n > 0)
}

pub fn delete_client(conn: &Connection, id: Id) -> Result<bool> {
    let n = conn.execute("DELETE FROM clients WHERE id = ?1", [text(id)])?;
    Ok(n > 0)
}

pub fn get_client(conn: &Connection, id: Id) -> Result<Option<Client>> {
    Ok(conn
        .query_row(
            &format!("SELECT {CLIENT_COLS} FROM clients WHERE id = ?1"),
            [text(id)],
            client_row,
        )
        .optional()?)
}

pub fn list_clients(conn: &Connection) -> Result<Vec<Client>> {
    let mut st = conn.prepare(&format!(
        "SELECT {CLIENT_COLS} FROM clients ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = st.query_map([], client_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- projects ----

const PROJECT_COLS: &str = "id, name, color, client_id, is_billable, billable_rate, \
                            estimated_time, archived_at, created_at, updated_at";

fn project_row(r: &Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: col_id(r, 0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        client_id: opt_id(r, 3)?,
        is_billable: r.get(4)?,
        billable_rate: r.get(5)?,
        estimated_time: r.get(6)?,
        archived_at: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

pub fn insert_project(conn: &Connection, p: &Project) -> Result<()> {
    conn.execute(
        &format!("INSERT INTO projects({PROJECT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"),
        params![
            text(p.id),
            p.name,
            p.color,
            opt_text(p.client_id),
            p.is_billable,
            p.billable_rate,
            p.estimated_time,
            p.archived_at,
            p.created_at,
            p.updated_at
        ],
    )?;
    Ok(())
}

pub fn update_project(conn: &Connection, p: &Project) -> Result<bool> {
    let n = conn.execute(
        "UPDATE projects SET name=?2, color=?3, client_id=?4, is_billable=?5, billable_rate=?6,
         estimated_time=?7, archived_at=?8, created_at=?9, updated_at=?10 WHERE id=?1",
        params![
            text(p.id),
            p.name,
            p.color,
            opt_text(p.client_id),
            p.is_billable,
            p.billable_rate,
            p.estimated_time,
            p.archived_at,
            p.created_at,
            p.updated_at
        ],
    )?;
    Ok(n > 0)
}

pub fn delete_project(conn: &Connection, id: Id) -> Result<bool> {
    let n = conn.execute("DELETE FROM projects WHERE id = ?1", [text(id)])?;
    Ok(n > 0)
}

pub fn get_project(conn: &Connection, id: Id) -> Result<Option<Project>> {
    Ok(conn
        .query_row(
            &format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1"),
            [text(id)],
            project_row,
        )
        .optional()?)
}

pub fn list_projects(conn: &Connection) -> Result<Vec<Project>> {
    let mut st = conn.prepare(&format!(
        "SELECT {PROJECT_COLS} FROM projects ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = st.query_map([], project_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- tasks ----

const TASK_COLS: &str = "id, project_id, name, estimated_time, done_at, created_at, updated_at";

fn task_row(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: col_id(r, 0)?,
        project_id: col_id(r, 1)?,
        name: r.get(2)?,
        estimated_time: r.get(3)?,
        done_at: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

pub fn insert_task(conn: &Connection, t: &Task) -> Result<()> {
    conn.execute(
        &format!("INSERT INTO tasks({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7)"),
        params![
            text(t.id),
            text(t.project_id),
            t.name,
            t.estimated_time,
            t.done_at,
            t.created_at,
            t.updated_at
        ],
    )?;
    Ok(())
}

pub fn update_task(conn: &Connection, t: &Task) -> Result<bool> {
    let n = conn.execute(
        "UPDATE tasks SET project_id=?2, name=?3, estimated_time=?4, done_at=?5,
         created_at=?6, updated_at=?7 WHERE id=?1",
        params![
            text(t.id),
            text(t.project_id),
            t.name,
            t.estimated_time,
            t.done_at,
            t.created_at,
            t.updated_at
        ],
    )?;
    Ok(n > 0)
}

pub fn delete_task(conn: &Connection, id: Id) -> Result<bool> {
    let n = conn.execute("DELETE FROM tasks WHERE id = ?1", [text(id)])?;
    Ok(n > 0)
}

pub fn get_task(conn: &Connection, id: Id) -> Result<Option<Task>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1"),
            [text(id)],
            task_row,
        )
        .optional()?)
}

pub fn list_tasks(conn: &Connection, project_id: Id) -> Result<Vec<Task>> {
    let mut st = conn.prepare(&format!(
        "SELECT {TASK_COLS} FROM tasks WHERE project_id = ?1 ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = st.query_map([text(project_id)], task_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- tags ----

const TAG_COLS: &str = "id, name, created_at, updated_at";

fn tag_row(r: &Row) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: col_id(r, 0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
    })
}

pub fn insert_tag(conn: &Connection, t: &Tag) -> Result<()> {
    conn.execute(
        "INSERT INTO tags(id, name, created_at, updated_at) VALUES (?1,?2,?3,?4)",
        params![text(t.id), t.name, t.created_at, t.updated_at],
    )?;
    Ok(())
}

pub fn update_tag(conn: &Connection, t: &Tag) -> Result<bool> {
    let n = conn.execute(
        "UPDATE tags SET name=?2, created_at=?3, updated_at=?4 WHERE id=?1",
        params![text(t.id), t.name, t.created_at, t.updated_at],
    )?;
    Ok(n > 0)
}

/// Cascades to `time_entry_tags`.
pub fn delete_tag(conn: &Connection, id: Id) -> Result<bool> {
    let n = conn.execute("DELETE FROM tags WHERE id = ?1", [text(id)])?;
    Ok(n > 0)
}

pub fn get_tag(conn: &Connection, id: Id) -> Result<Option<Tag>> {
    Ok(conn
        .query_row(
            &format!("SELECT {TAG_COLS} FROM tags WHERE id = ?1"),
            [text(id)],
            tag_row,
        )
        .optional()?)
}

pub fn list_tags(conn: &Connection) -> Result<Vec<Tag>> {
    let mut st = conn.prepare(&format!(
        "SELECT {TAG_COLS} FROM tags ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = st.query_map([], tag_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---- counts ----

fn count(conn: &Connection, sql: &str, id: Id) -> Result<i64> {
    Ok(conn.query_row(sql, [text(id)], |r| r.get(0))?)
}

pub fn count_entries_for_project(conn: &Connection, id: Id) -> Result<i64> {
    count(
        conn,
        "SELECT COUNT(*) FROM time_entries WHERE project_id = ?1",
        id,
    )
}

pub fn count_entries_for_task(conn: &Connection, id: Id) -> Result<i64> {
    count(
        conn,
        "SELECT COUNT(*) FROM time_entries WHERE task_id = ?1",
        id,
    )
}

/// Time entries that carry the tag.
pub fn count_entries_for_tag(conn: &Connection, id: Id) -> Result<i64> {
    count(
        conn,
        "SELECT COUNT(*) FROM time_entry_tags WHERE tag_id = ?1",
        id,
    )
}

pub fn count_projects_for_client(conn: &Connection, id: Id) -> Result<i64> {
    count(
        conn,
        "SELECT COUNT(*) FROM projects WHERE client_id = ?1",
        id,
    )
}

pub fn count_tasks_for_project(conn: &Connection, id: Id) -> Result<i64> {
    count(conn, "SELECT COUNT(*) FROM tasks WHERE project_id = ?1", id)
}
