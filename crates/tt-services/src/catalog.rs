//! Clients, projects, tasks and tags: validation, usage checks on delete, stats.

use tt_domain::{Client, Id, Millis, Project, Tag, Task, new_id};
use tt_storage::{Db, catalog, entries};

use crate::clock::Clock;
use crate::error::{Result, ServiceError};
use crate::tx::in_tx;

const MAX_NAME: usize = 255;

/// Fields of a project to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProject {
    pub name: String,
    /// "#rrggbb"
    pub color: String,
    pub client_id: Option<Id>,
    pub is_billable: bool,
    /// Cents per hour.
    pub billable_rate: Option<i64>,
    /// Seconds.
    pub estimated_time: Option<i64>,
}

pub struct CatalogService<C: Clock> {
    clock: C,
}

impl<C: Clock> CatalogService<C> {
    pub fn new(clock: C) -> Self {
        Self { clock }
    }

    // ---- clients ----

    pub fn create_client(&self, db: &mut Db, name: &str) -> Result<Client> {
        let name = valid_name(name)?;
        let now = self.clock.now();
        let client = Client {
            id: new_id(),
            name,
            archived_at: None,
            created_at: now,
            updated_at: now,
        };
        in_tx(db, |c| Ok(catalog::insert_client(c, &client)?))?;
        Ok(client)
    }

    pub fn rename_client(&self, db: &mut Db, id: Id, name: &str) -> Result<Client> {
        let name = valid_name(name)?;
        self.change_client(db, id, |cl, _| cl.name = name)
    }

    pub fn set_client_archived(&self, db: &mut Db, id: Id, archived: bool) -> Result<Client> {
        self.change_client(db, id, |cl, now| cl.archived_at = archived.then_some(now))
    }

    pub fn delete_client(&self, db: &mut Db, id: Id) -> Result<()> {
        in_tx(db, |c| {
            if catalog::count_projects_for_client(c, id)? > 0 {
                return Err(ServiceError::InUse { what: "projects" });
            }
            found(catalog::delete_client(c, id)?)
        })
    }

    pub fn list_clients(&self, db: &Db) -> Result<Vec<Client>> {
        Ok(catalog::list_clients(db.conn())?)
    }

    fn change_client(
        &self,
        db: &mut Db,
        id: Id,
        f: impl FnOnce(&mut Client, Millis),
    ) -> Result<Client> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut cl = catalog::get_client(c, id)?.ok_or(ServiceError::NotFound)?;
            f(&mut cl, now);
            cl.updated_at = now;
            found(catalog::update_client(c, &cl)?)?;
            Ok(cl)
        })
    }

    // ---- projects ----

    pub fn create_project(&self, db: &mut Db, p: NewProject) -> Result<Project> {
        let name = valid_name(&p.name)?;
        valid_color(&p.color)?;
        let now = self.clock.now();
        let project = Project {
            id: new_id(),
            name,
            color: p.color,
            client_id: p.client_id,
            is_billable: p.is_billable,
            billable_rate: p.billable_rate,
            estimated_time: p.estimated_time,
            archived_at: None,
            created_at: now,
            updated_at: now,
        };
        in_tx(db, |c| {
            if let Some(cid) = project.client_id {
                catalog::get_client(c, cid)?.ok_or(ServiceError::NotFound)?;
            }
            Ok(catalog::insert_project(c, &project)?)
        })?;
        Ok(project)
    }

    /// Saves the editable fields; `created_at` and `archived_at` stay as stored.
    pub fn update_project(&self, db: &mut Db, p: &Project) -> Result<Project> {
        let mut p = p.clone();
        p.name = valid_name(&p.name)?;
        valid_color(&p.color)?;
        p.updated_at = self.clock.now();
        in_tx(db, |c| {
            let stored = catalog::get_project(c, p.id)?.ok_or(ServiceError::NotFound)?;
            p.created_at = stored.created_at;
            p.archived_at = stored.archived_at;
            if let Some(cid) = p.client_id {
                catalog::get_client(c, cid)?.ok_or(ServiceError::NotFound)?;
            }
            found(catalog::update_project(c, &p)?)
        })?;
        Ok(p)
    }

    pub fn set_project_archived(&self, db: &mut Db, id: Id, archived: bool) -> Result<Project> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut p = catalog::get_project(c, id)?.ok_or(ServiceError::NotFound)?;
            p.archived_at = archived.then_some(now);
            p.updated_at = now;
            found(catalog::update_project(c, &p)?)?;
            Ok(p)
        })
    }

    pub fn delete_project(&self, db: &mut Db, id: Id) -> Result<()> {
        in_tx(db, |c| {
            if catalog::count_tasks_for_project(c, id)? > 0 {
                return Err(ServiceError::InUse { what: "tasks" });
            }
            if catalog::count_entries_for_project(c, id)? > 0 {
                return Err(ServiceError::InUse {
                    what: "time_entries",
                });
            }
            found(catalog::delete_project(c, id)?)
        })
    }

    pub fn list_projects(&self, db: &Db) -> Result<Vec<Project>> {
        Ok(catalog::list_projects(db.conn())?)
    }

    /// (spent ms with a running entry counted up to `now`, estimate in seconds).
    pub fn project_stats(&self, db: &Db, id: Id, now: Millis) -> Result<(i64, Option<i64>)> {
        let p = catalog::get_project(db.conn(), id)?.ok_or(ServiceError::NotFound)?;
        let spent = entries::spent_ms_for_project(db.conn(), id, now)?;
        Ok((spent, p.estimated_time))
    }

    // ---- tasks ----

    pub fn create_task(
        &self,
        db: &mut Db,
        project_id: Id,
        name: &str,
        estimated_time: Option<i64>,
    ) -> Result<Task> {
        let name = valid_name(name)?;
        let now = self.clock.now();
        let task = Task {
            id: new_id(),
            project_id,
            name,
            estimated_time,
            done_at: None,
            created_at: now,
            updated_at: now,
        };
        in_tx(db, |c| {
            catalog::get_project(c, project_id)?.ok_or(ServiceError::NotFound)?;
            Ok(catalog::insert_task(c, &task)?)
        })?;
        Ok(task)
    }

    /// Saves name, estimate and project; `created_at` and `done_at` stay as stored.
    /// Moving a task that has entries to another project is refused.
    pub fn update_task(&self, db: &mut Db, t: &Task) -> Result<Task> {
        let mut t = t.clone();
        t.name = valid_name(&t.name)?;
        t.updated_at = self.clock.now();
        in_tx(db, |c| {
            let stored = catalog::get_task(c, t.id)?.ok_or(ServiceError::NotFound)?;
            t.created_at = stored.created_at;
            t.done_at = stored.done_at;
            if t.project_id != stored.project_id {
                catalog::get_project(c, t.project_id)?.ok_or(ServiceError::NotFound)?;
                if catalog::count_entries_for_task(c, t.id)? > 0 {
                    return Err(ServiceError::InUse {
                        what: "time_entries",
                    });
                }
            }
            found(catalog::update_task(c, &t)?)
        })?;
        Ok(t)
    }

    pub fn set_task_done(&self, db: &mut Db, id: Id, done: bool) -> Result<Task> {
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut t = catalog::get_task(c, id)?.ok_or(ServiceError::NotFound)?;
            t.done_at = done.then_some(now);
            t.updated_at = now;
            found(catalog::update_task(c, &t)?)?;
            Ok(t)
        })
    }

    pub fn delete_task(&self, db: &mut Db, id: Id) -> Result<()> {
        in_tx(db, |c| {
            if catalog::count_entries_for_task(c, id)? > 0 {
                return Err(ServiceError::InUse {
                    what: "time_entries",
                });
            }
            found(catalog::delete_task(c, id)?)
        })
    }

    pub fn list_tasks(&self, db: &Db, project_id: Id) -> Result<Vec<Task>> {
        Ok(catalog::list_tasks(db.conn(), project_id)?)
    }

    // ---- tags ----

    pub fn create_tag(&self, db: &mut Db, name: &str) -> Result<Tag> {
        let name = valid_name(name)?;
        let now = self.clock.now();
        let tag = Tag {
            id: new_id(),
            name,
            created_at: now,
            updated_at: now,
        };
        in_tx(db, |c| Ok(catalog::insert_tag(c, &tag)?))?;
        Ok(tag)
    }

    pub fn rename_tag(&self, db: &mut Db, id: Id, name: &str) -> Result<Tag> {
        let name = valid_name(name)?;
        let now = self.clock.now();
        in_tx(db, |c| {
            let mut t = catalog::get_tag(c, id)?.ok_or(ServiceError::NotFound)?;
            t.name = name;
            t.updated_at = now;
            found(catalog::update_tag(c, &t)?)?;
            Ok(t)
        })
    }

    /// Removes the tag; entries lose it.
    pub fn delete_tag(&self, db: &mut Db, id: Id) -> Result<()> {
        in_tx(db, |c| found(catalog::delete_tag(c, id)?))
    }

    pub fn list_tags(&self, db: &Db) -> Result<Vec<Tag>> {
        Ok(catalog::list_tags(db.conn())?)
    }
}

/// Repo "affected rows" flag to `NotFound`.
fn found(changed: bool) -> Result<()> {
    if changed {
        Ok(())
    } else {
        Err(ServiceError::NotFound)
    }
}

/// Trimmed, non-empty name of at most 255 chars.
fn valid_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME {
        return Err(ServiceError::Invalid("name"));
    }
    Ok(name.to_owned())
}

/// `#` followed by six hex digits.
fn valid_color(color: &str) -> Result<()> {
    let ok = color
        .strip_prefix('#')
        .is_some_and(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    if ok {
        Ok(())
    } else {
        Err(ServiceError::Invalid("color"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use tt_domain::TimeEntry;

    fn setup() -> (CatalogService<FixedClock>, Db) {
        (
            CatalogService::new(FixedClock::new(1_000)),
            Db::open_in_memory().unwrap(),
        )
    }

    fn np(name: &str) -> NewProject {
        NewProject {
            name: name.into(),
            color: "#aabbcc".into(),
            client_id: None,
            is_billable: true,
            billable_rate: Some(5000),
            estimated_time: Some(3600),
        }
    }

    fn entry(project: Option<Id>, task: Option<Id>, tags: Vec<Id>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: String::new(),
            start: 0,
            end: Some(10_000),
            billable: false,
            project_id: project,
            task_id: task,
            tag_ids: tags,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn name_is_trimmed_and_validated() {
        let (svc, mut db) = setup();
        assert_eq!(svc.create_client(&mut db, "  Acme ").unwrap().name, "Acme");
        for bad in ["", "   ", &"x".repeat(256)] {
            assert!(matches!(
                svc.create_client(&mut db, bad),
                Err(ServiceError::Invalid("name"))
            ));
            assert!(matches!(
                svc.create_project(&mut db, np(bad)),
                Err(ServiceError::Invalid("name"))
            ));
            assert!(matches!(
                svc.create_tag(&mut db, bad),
                Err(ServiceError::Invalid("name"))
            ));
        }
        assert!(svc.create_tag(&mut db, &"я".repeat(255)).is_ok());
        let c = svc.create_client(&mut db, "A").unwrap();
        assert!(matches!(
            svc.rename_client(&mut db, c.id, " "),
            Err(ServiceError::Invalid("name"))
        ));
        assert_eq!(svc.rename_client(&mut db, c.id, " B ").unwrap().name, "B");
    }

    #[test]
    fn color_must_be_hex_rgb() {
        let (svc, mut db) = setup();
        for ok in ["#000000", "#AbCdEf", "#0a1B2c"] {
            let mut p = np("p");
            p.color = ok.into();
            assert!(svc.create_project(&mut db, p).is_ok(), "{ok}");
        }
        for bad in ["", "aabbcc", "#abc", "#aabbccd", "#gghhii", "# aabbc"] {
            let mut p = np("p");
            p.color = bad.into();
            assert!(
                matches!(
                    svc.create_project(&mut db, p),
                    Err(ServiceError::Invalid("color"))
                ),
                "{bad}"
            );
        }
        let mut p = svc.create_project(&mut db, np("q")).unwrap();
        p.color = "red".into();
        assert!(matches!(
            svc.update_project(&mut db, &p),
            Err(ServiceError::Invalid("color"))
        ));
    }

    #[test]
    fn create_update_archive_roundtrip() {
        let (svc, mut db) = setup();
        let cl = svc.create_client(&mut db, "Acme").unwrap();
        let mut input = np(" P ");
        input.client_id = Some(cl.id);
        let mut p = svc.create_project(&mut db, input).unwrap();
        assert_eq!((p.name.as_str(), p.created_at), ("P", 1_000));
        svc.clock.set(2_000);
        p.name = "P2".into();
        let saved = svc.update_project(&mut db, &p).unwrap();
        assert_eq!(saved.updated_at, 2_000);
        let archived = svc.set_project_archived(&mut db, p.id, true).unwrap();
        assert_eq!(archived.archived_at, Some(2_000));
        assert_eq!(svc.list_projects(&db).unwrap(), vec![archived]);
        let c = svc.set_client_archived(&mut db, cl.id, true).unwrap();
        assert_eq!(c.archived_at, Some(2_000));
        let c = svc.set_client_archived(&mut db, cl.id, false).unwrap();
        assert_eq!(c.archived_at, None);

        let t = svc.create_task(&mut db, p.id, "T", Some(60)).unwrap();
        let t = svc.set_task_done(&mut db, t.id, true).unwrap();
        assert_eq!(t.done_at, Some(2_000));
        assert_eq!(svc.list_tasks(&db, p.id).unwrap(), vec![t]);
    }

    #[test]
    fn stale_update_does_not_unarchive_or_reopen() {
        let (svc, mut db) = setup();
        let p = svc.create_project(&mut db, np("p")).unwrap();
        let t = svc.create_task(&mut db, p.id, "t", None).unwrap();
        svc.clock.set(2_000);
        svc.set_project_archived(&mut db, p.id, true).unwrap();
        svc.set_task_done(&mut db, t.id, true).unwrap();
        svc.clock.set(3_000);
        let mut stale_p = p.clone();
        stale_p.name = "renamed".into();
        stale_p.created_at = 42;
        let saved = svc.update_project(&mut db, &stale_p).unwrap();
        assert_eq!((saved.archived_at, saved.created_at), (Some(2_000), 1_000));
        let mut stale_t = t.clone();
        stale_t.name = "renamed".into();
        let saved = svc.update_task(&mut db, &stale_t).unwrap();
        assert_eq!((saved.done_at, saved.created_at), (Some(2_000), 1_000));
        let stored = catalog::get_task(db.conn(), t.id).unwrap().unwrap();
        assert_eq!(stored, saved);
        let sp = catalog::get_project(db.conn(), p.id).unwrap().unwrap();
        assert_eq!((sp.name.as_str(), sp.archived_at), ("renamed", Some(2_000)));
    }

    #[test]
    fn moving_task_with_entries_is_refused() {
        let (svc, mut db) = setup();
        let p1 = svc.create_project(&mut db, np("a")).unwrap();
        let p2 = svc.create_project(&mut db, np("b")).unwrap();
        let t = svc.create_task(&mut db, p1.id, "t", None).unwrap();
        let mut moved = t.clone();
        moved.project_id = p2.id;
        assert_eq!(svc.update_task(&mut db, &moved).unwrap().project_id, p2.id);
        moved.project_id = p1.id;
        entries::insert_entry(db.conn(), &entry(Some(p2.id), Some(t.id), vec![])).unwrap();
        assert!(matches!(
            svc.update_task(&mut db, &moved),
            Err(ServiceError::InUse {
                what: "time_entries"
            })
        ));
        moved.project_id = new_id();
        assert!(matches!(
            svc.update_task(&mut db, &moved),
            Err(ServiceError::NotFound)
        ));
        let stored = catalog::get_task(db.conn(), t.id).unwrap().unwrap();
        assert_eq!(stored.project_id, p2.id);
    }

    #[test]
    fn missing_ids_are_not_found() {
        let (svc, mut db) = setup();
        let id = new_id();
        assert!(matches!(
            svc.create_task(&mut db, id, "t", None),
            Err(ServiceError::NotFound)
        ));
        let mut p = np("p");
        p.client_id = Some(id);
        assert!(matches!(
            svc.create_project(&mut db, p),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.rename_client(&mut db, id, "x"),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.delete_client(&mut db, id),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.delete_project(&mut db, id),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.delete_task(&mut db, id),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.delete_tag(&mut db, id),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.set_task_done(&mut db, id, true),
            Err(ServiceError::NotFound)
        ));
        assert!(matches!(
            svc.project_stats(&db, id, 0),
            Err(ServiceError::NotFound)
        ));
    }

    #[test]
    fn delete_client_with_projects_is_in_use() {
        let (svc, mut db) = setup();
        let cl = svc.create_client(&mut db, "Acme").unwrap();
        let mut input = np("p");
        input.client_id = Some(cl.id);
        let p = svc.create_project(&mut db, input).unwrap();
        assert!(matches!(
            svc.delete_client(&mut db, cl.id),
            Err(ServiceError::InUse { what: "projects" })
        ));
        svc.delete_project(&mut db, p.id).unwrap();
        svc.delete_client(&mut db, cl.id).unwrap();
        assert!(svc.list_clients(&db).unwrap().is_empty());
    }

    #[test]
    fn delete_project_checks_tasks_then_entries() {
        let (svc, mut db) = setup();
        let p = svc.create_project(&mut db, np("p")).unwrap();
        let t = svc.create_task(&mut db, p.id, "t", None).unwrap();
        entries::insert_entry(db.conn(), &entry(Some(p.id), Some(t.id), vec![])).unwrap();
        assert!(matches!(
            svc.delete_project(&mut db, p.id),
            Err(ServiceError::InUse { what: "tasks" })
        ));
        assert!(matches!(
            svc.delete_task(&mut db, t.id),
            Err(ServiceError::InUse {
                what: "time_entries"
            })
        ));
        // Entry stays on the project, task removed from the entry.
        db.conn()
            .execute("UPDATE time_entries SET task_id = NULL", [])
            .unwrap();
        svc.delete_task(&mut db, t.id).unwrap();
        assert!(matches!(
            svc.delete_project(&mut db, p.id),
            Err(ServiceError::InUse {
                what: "time_entries"
            })
        ));
        db.conn().execute("DELETE FROM time_entries", []).unwrap();
        svc.delete_project(&mut db, p.id).unwrap();
    }

    #[test]
    fn delete_tag_unlinks_entries() {
        let (svc, mut db) = setup();
        let tag = svc.create_tag(&mut db, "x").unwrap();
        let e = entry(None, None, vec![tag.id]);
        entries::insert_entry(db.conn(), &e).unwrap();
        svc.delete_tag(&mut db, tag.id).unwrap();
        let got = entries::get_entry(db.conn(), e.id).unwrap().unwrap();
        assert!(got.tag_ids.is_empty());
        assert!(svc.list_tags(&db).unwrap().is_empty());
    }

    #[test]
    fn project_stats_counts_running_entry() {
        let (svc, mut db) = setup();
        let p = svc.create_project(&mut db, np("p")).unwrap();
        assert_eq!(svc.project_stats(&db, p.id, 0).unwrap(), (0, Some(3600)));
        let mut closed = entry(Some(p.id), None, vec![]);
        closed.start = 0;
        closed.end = Some(60_000);
        entries::insert_entry(db.conn(), &closed).unwrap();
        let mut running = entry(Some(p.id), None, vec![]);
        running.start = 100_000;
        running.end = None;
        entries::insert_entry(db.conn(), &running).unwrap();
        assert_eq!(
            svc.project_stats(&db, p.id, 130_000).unwrap(),
            (90_000, Some(3600))
        );
    }
}
