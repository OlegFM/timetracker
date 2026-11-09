use tt_domain::{
    Client, EntryTemplate, Id, Project, SessionState, Tag, Task, TimeEntry, TimerSession, new_id,
};
use tt_storage::{Db, SCHEMA_VERSION, StorageError, catalog, entries, settings};

fn client(name: &str) -> Client {
    Client {
        id: new_id(),
        name: name.into(),
        archived_at: None,
        created_at: 1,
        updated_at: 2,
    }
}

fn project(name: &str, client_id: Option<Id>) -> Project {
    Project {
        id: new_id(),
        name: name.into(),
        color: "#aabbcc".into(),
        client_id,
        is_billable: true,
        billable_rate: Some(5000),
        estimated_time: Some(3600),
        archived_at: None,
        created_at: 1,
        updated_at: 2,
    }
}

fn task(project_id: Id, name: &str) -> Task {
    Task {
        id: new_id(),
        project_id,
        name: name.into(),
        estimated_time: None,
        done_at: Some(9),
        created_at: 1,
        updated_at: 2,
    }
}

fn tag(name: &str) -> Tag {
    Tag {
        id: new_id(),
        name: name.into(),
        created_at: 1,
        updated_at: 2,
    }
}

fn entry(start: i64, end: Option<i64>) -> TimeEntry {
    TimeEntry {
        id: new_id(),
        session_id: None,
        description: "work".into(),
        start,
        end,
        billable: true,
        project_id: None,
        task_id: None,
        tag_ids: vec![],
        created_at: 1,
        updated_at: 2,
    }
}

fn user_version(db: &Db) -> i32 {
    db.conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn migrations_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    drop(Db::open(&path).unwrap());
    let db = Db::open(&path).unwrap();
    assert_eq!(user_version(&db), 1);
    assert_eq!(SCHEMA_VERSION, 1);
    let fk: i32 = db
        .conn()
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
}

#[test]
fn client_crud_and_ordering() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let mut b = client("beta");
    let a = client("Alpha");
    catalog::insert_client(c, &b).unwrap();
    catalog::insert_client(c, &a).unwrap();
    assert_eq!(catalog::get_client(c, a.id).unwrap().unwrap(), a);
    let names: Vec<_> = catalog::list_clients(c)
        .unwrap()
        .into_iter()
        .map(|x| x.name)
        .collect();
    assert_eq!(names, ["Alpha", "beta"]);
    b.name = "zeta".into();
    b.archived_at = Some(5);
    catalog::update_client(c, &b).unwrap();
    assert_eq!(catalog::get_client(c, b.id).unwrap().unwrap(), b);
    catalog::delete_client(c, b.id).unwrap();
    assert!(catalog::get_client(c, b.id).unwrap().is_none());
}

#[test]
fn project_task_tag_crud_and_counts() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let cl = client("c");
    catalog::insert_client(c, &cl).unwrap();
    let mut p = project("p", Some(cl.id));
    catalog::insert_project(c, &p).unwrap();
    assert_eq!(catalog::get_project(c, p.id).unwrap().unwrap(), p);
    p.name = "p2".into();
    p.billable_rate = None;
    p.is_billable = false;
    catalog::update_project(c, &p).unwrap();
    assert_eq!(catalog::list_projects(c).unwrap(), vec![p.clone()]);
    assert_eq!(catalog::count_projects_for_client(c, cl.id).unwrap(), 1);

    let t2 = task(p.id, "b-task");
    let mut t1 = task(p.id, "A-task");
    catalog::insert_task(c, &t2).unwrap();
    catalog::insert_task(c, &t1).unwrap();
    assert_eq!(catalog::get_task(c, t1.id).unwrap().unwrap(), t1);
    t1.done_at = None;
    t1.name = "c-task".into();
    catalog::update_task(c, &t1).unwrap();
    let names: Vec<_> = catalog::list_tasks(c, p.id)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["b-task", "c-task"]);
    assert_eq!(catalog::count_tasks_for_project(c, p.id).unwrap(), 2);

    let mut g = tag("g");
    catalog::insert_tag(c, &g).unwrap();
    g.name = "h".into();
    catalog::update_tag(c, &g).unwrap();
    assert_eq!(catalog::get_tag(c, g.id).unwrap().unwrap(), g);
    assert_eq!(catalog::list_tags(c).unwrap(), vec![g.clone()]);
    catalog::delete_tag(c, g.id).unwrap();
    assert!(catalog::list_tags(c).unwrap().is_empty());

    catalog::delete_task(c, t1.id).unwrap();
    catalog::delete_task(c, t2.id).unwrap();
    catalog::delete_project(c, p.id).unwrap();
    assert!(catalog::get_project(c, p.id).unwrap().is_none());
}

#[test]
fn entry_round_trip_with_tags() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let p = project("p", None);
    catalog::insert_project(c, &p).unwrap();
    let t = task(p.id, "t");
    catalog::insert_task(c, &t).unwrap();
    let (g1, g2) = (tag("one"), tag("two"));
    catalog::insert_tag(c, &g1).unwrap();
    catalog::insert_tag(c, &g2).unwrap();

    let mut e = entry(100, Some(200));
    e.project_id = Some(p.id);
    e.task_id = Some(t.id);
    // duplicates are collapsed, output is sorted ascending
    e.tag_ids = vec![g2.id, g1.id, g2.id];
    entries::insert_entry(c, &e).unwrap();
    let mut sorted = vec![g1.id, g2.id];
    sorted.sort();
    let got = entries::get_entry(c, e.id).unwrap().unwrap();
    assert_eq!(got.tag_ids, sorted);
    assert_eq!(
        TimeEntry {
            tag_ids: vec![],
            ..got
        },
        TimeEntry {
            tag_ids: vec![],
            ..e.clone()
        }
    );

    e.tag_ids = vec![g1.id];
    e.description = "changed".into();
    e.end = Some(300);
    entries::update_entry(c, &e).unwrap();
    assert_eq!(entries::get_entry(c, e.id).unwrap().unwrap(), e);

    assert_eq!(catalog::count_entries_for_project(c, p.id).unwrap(), 1);
    assert_eq!(catalog::count_entries_for_task(c, t.id).unwrap(), 1);
    assert_eq!(catalog::count_entries_for_tag(c, g1.id).unwrap(), 1);
    assert_eq!(catalog::count_entries_for_tag(c, g2.id).unwrap(), 0);
    let counts = entries::entry_counts_by_tag(c).unwrap();
    assert_eq!(counts.get(&g1.id), Some(&1));
    assert_eq!(counts.get(&g2.id), None);

    // deleting a tag cascades to the link table
    catalog::delete_tag(c, g1.id).unwrap();
    assert!(
        entries::get_entry(c, e.id)
            .unwrap()
            .unwrap()
            .tag_ids
            .is_empty()
    );

    entries::delete_entry(c, e.id).unwrap();
    assert!(entries::get_entry(c, e.id).unwrap().is_none());
}

#[test]
fn only_one_running_entry() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    entries::insert_entry(c, &entry(10, None)).unwrap();
    entries::insert_entry(c, &entry(1, Some(2))).unwrap();
    let err = entries::insert_entry(c, &entry(20, None)).unwrap_err();
    assert!(matches!(err, StorageError::Sqlite(_)), "{err:?}");
}

#[test]
fn entries_in_range_includes_overlapping_and_running() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let before = entry(0, Some(10)); // touches range start: excluded
    let straddle_start = entry(5, Some(15));
    let inside = entry(20, Some(25));
    let straddle_end = entry(28, Some(40));
    let after = entry(30, Some(35)); // starts at range end: excluded
    let running = entry(22, None);
    for e in [
        &before,
        &straddle_start,
        &inside,
        &straddle_end,
        &after,
        &running,
    ] {
        entries::insert_entry(c, e).unwrap();
    }
    let ids: Vec<_> = entries::entries_in_range(c, 10, 30)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        ids,
        [straddle_start.id, inside.id, running.id, straddle_end.id]
    );
}

#[test]
fn overlapping_excludes_self_and_touching() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let a = entry(10, Some(11));
    entries::insert_entry(c, &a).unwrap();
    assert!(!entries::overlapping(c, 11, 12, None).unwrap());
    assert!(!entries::overlapping(c, 9, 10, None).unwrap());
    assert!(entries::overlapping(c, 10, 11, None).unwrap());
    assert!(!entries::overlapping(c, 10, 11, Some(a.id)).unwrap());
    let run = entry(50, None);
    entries::insert_entry(c, &run).unwrap();
    assert!(entries::overlapping(c, 100, 101, None).unwrap());
    assert!(!entries::overlapping(c, 100, 101, Some(run.id)).unwrap());
}

#[test]
fn recent_and_session_entries_and_spent() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let s = TimerSession {
        id: new_id(),
        started_at: 0,
        ended_at: None,
        state: SessionState::Running,
    };
    entries::insert_session(c, &s).unwrap();
    let p = project("p", None);
    catalog::insert_project(c, &p).unwrap();
    let t = task(p.id, "t");
    catalog::insert_task(c, &t).unwrap();

    let mut e1 = entry(0, Some(100));
    let mut e2 = entry(200, None);
    let e3 = entry(300, Some(310));
    e1.session_id = Some(s.id);
    e2.session_id = Some(s.id);
    e1.project_id = Some(p.id);
    e2.project_id = Some(p.id);
    e2.task_id = Some(t.id);
    for e in [&e1, &e2] {
        entries::insert_entry(c, e).unwrap();
    }
    entries::insert_entry(c, &e3).unwrap();

    let recent: Vec<_> = entries::recent_entries(c, 2)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(recent, [e3.id, e2.id]);
    let of_session: Vec<_> = entries::entries_of_session(c, s.id)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(of_session, [e1.id, e2.id]);

    assert_eq!(entries::spent_ms_for_project(c, p.id, 250).unwrap(), 150);
    assert_eq!(entries::spent_ms_for_task(c, t.id, 250).unwrap(), 50);
    assert_eq!(entries::spent_ms_for_task(c, new_id(), 250).unwrap(), 0);

    // grouped variants agree with the single-id ones (incl. a running entry up to `now`)
    let by_project = entries::spent_ms_by_project(c, 250).unwrap();
    assert_eq!(by_project[&p.id], 150);
    assert_eq!(by_project.len(), 1);
    let by_task = entries::spent_ms_by_task(c, p.id, 250).unwrap();
    assert_eq!(by_task[&t.id], 50);
    assert!(
        entries::spent_ms_by_task(c, new_id(), 250)
            .unwrap()
            .is_empty()
    );

    // deleting the session detaches entries (SET NULL)
    c.execute("DELETE FROM timer_sessions", []).unwrap();
    assert_eq!(
        entries::get_entry(c, e1.id).unwrap().unwrap().session_id,
        None
    );
}

#[test]
fn session_active_and_template() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    assert!(entries::active_session(c).unwrap().is_none());
    let mut s = TimerSession {
        id: new_id(),
        started_at: 5,
        ended_at: None,
        state: SessionState::Paused,
    };
    entries::insert_session(c, &s).unwrap();
    assert_eq!(entries::active_session(c).unwrap().unwrap(), s);
    assert!(entries::session_template(c, s.id).unwrap().is_none());
    let tpl = EntryTemplate {
        description: "d".into(),
        project_id: Some(new_id()),
        task_id: None,
        tag_ids: vec![new_id()],
        billable: true,
    };
    entries::set_session_template(c, s.id, &tpl).unwrap();
    assert_eq!(entries::session_template(c, s.id).unwrap().unwrap(), tpl);
    s.state = SessionState::Stopped;
    s.ended_at = Some(9);
    entries::update_session(c, &s).unwrap();
    assert!(entries::active_session(c).unwrap().is_none());
    // update_session keeps the template
    assert_eq!(entries::session_template(c, s.id).unwrap().unwrap(), tpl);
}

#[test]
fn settings_get_set_upsert() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    assert_eq!(settings::get(c, "k").unwrap(), None);
    settings::set(c, "k", "v1").unwrap();
    settings::set(c, "k", "v2").unwrap();
    assert_eq!(settings::get(c, "k").unwrap().as_deref(), Some("v2"));
}

#[test]
fn delete_project_with_entries_is_restricted() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let p = project("p", None);
    catalog::insert_project(c, &p).unwrap();
    let mut e = entry(1, Some(2));
    e.project_id = Some(p.id);
    entries::insert_entry(c, &e).unwrap();
    assert!(catalog::delete_project(c, p.id).is_err());
    assert!(catalog::get_project(c, p.id).unwrap().is_some());
    let t = task(p.id, "t");
    catalog::insert_task(c, &t).unwrap();
    assert!(catalog::delete_project(c, p.id).is_err());
    let cl = client("c");
    catalog::insert_client(c, &cl).unwrap();
    let p2 = project("p2", Some(cl.id));
    catalog::insert_project(c, &p2).unwrap();
    assert!(catalog::delete_client(c, cl.id).is_err());
}

#[test]
fn transaction_commits_and_rolls_back() {
    let mut db = Db::open_in_memory().unwrap();
    let kept = client("kept");
    db.transaction(|tx| catalog::insert_client(tx, &kept))
        .unwrap();
    let dropped = client("dropped");
    let r: Result<(), _> = db.transaction(|tx| {
        catalog::insert_client(tx, &dropped)?;
        Err(StorageError::Corrupt("boom".into()))
    });
    assert!(r.is_err());
    let names: Vec<_> = catalog::list_clients(db.conn())
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["kept"]);
}

#[test]
fn corrupt_id_is_reported() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    c.execute(
        "INSERT INTO clients(id, name, created_at, updated_at) VALUES ('nope','x',1,1)",
        [],
    )
    .unwrap();
    assert!(matches!(
        catalog::list_clients(c),
        Err(StorageError::Corrupt(_))
    ));
}

#[test]
fn recover_running_entry_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let e = entry(1_000, None);
    {
        let db = Db::open(&path).unwrap();
        entries::insert_entry(db.conn(), &e).unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(entries::running_entry(db.conn()).unwrap().unwrap(), e);
}

#[test]
fn entry_writes_are_atomic_on_unknown_tag() {
    let db = Db::open_in_memory().unwrap();
    let c = db.conn();
    let g = tag("g");
    catalog::insert_tag(c, &g).unwrap();

    let mut bad = entry(1, Some(2));
    bad.tag_ids = vec![g.id, new_id()];
    assert!(entries::insert_entry(c, &bad).is_err());
    assert!(entries::get_entry(c, bad.id).unwrap().is_none());

    let mut e = entry(10, Some(20));
    e.tag_ids = vec![g.id];
    entries::insert_entry(c, &e).unwrap();
    let mut changed = e.clone();
    changed.description = "changed".into();
    changed.tag_ids = vec![new_id()];
    assert!(entries::update_entry(c, &changed).is_err());
    assert_eq!(entries::get_entry(c, e.id).unwrap().unwrap(), e);
}

#[test]
fn update_and_delete_report_whether_a_row_changed() {
    let mut db = Db::open_in_memory().unwrap();
    let missing = new_id();
    let c = db.conn();
    assert!(!catalog::delete_client(c, missing).unwrap());
    assert!(!catalog::update_client(c, &client("x")).unwrap());
    assert!(!catalog::delete_project(c, missing).unwrap());
    assert!(!catalog::delete_task(c, missing).unwrap());
    assert!(!catalog::delete_tag(c, missing).unwrap());
    assert!(!entries::delete_entry(c, missing).unwrap());
    assert!(!entries::update_entry(c, &entry(1, Some(2))).unwrap());
    let cl = client("c");
    catalog::insert_client(c, &cl).unwrap();
    assert!(catalog::update_client(c, &cl).unwrap());
    assert!(catalog::delete_client(c, cl.id).unwrap());
    // savepoints nest inside Db::transaction
    let e = entry(1, Some(2));
    db.transaction(|tx| {
        entries::insert_entry(tx, &e)?;
        assert!(entries::update_entry(tx, &e)?);
        Ok(())
    })
    .unwrap();
}
