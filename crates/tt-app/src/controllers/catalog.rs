//! Projects, the project page (tasks), clients and tags screens.
//!
//! The controller keeps the stored catalog (and the open project's tasks) and shows it
//! through `CatalogBridge`. After every change it reloads, refreshes the timer pickers
//! (R23) and tells the other listeners (labels in the entries list). Starting a task goes
//! through the [`TimerHub`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tt_domain::{Client, EntryTemplate, Id, Millis, Project, Tag, Task};
use tt_services::{Clock, NewProject, Result, ServiceError, default_billable};
use tt_storage::{catalog, entries};

use crate::app::{AppContext, Ctx};
use crate::controllers::timer::TimerHub;
use crate::controllers::{error_message, pickers::model, sync_vec};
use crate::vm::catalog::{
    client_row, estimate_text, money_text, parse_estimate, parse_rate, project_row, sorted_tasks,
    tag_row, task_row, visible_clients, visible_projects,
};
use crate::vm::error::{ErrorText, error_code, error_text};
use crate::vm::format::prefs_from;
use crate::vm::pickers::{PALETTE, hex_color, palette_color, parse_id};
use crate::{
    CatalogBridge, ClientRowView, MainWindow, ProjectDialogData, ProjectOption, ProjectRowView,
    TagRowView, TaskDialogData, TaskRowView, ToastKind, Tr,
};

/// The stored catalog as last loaded.
#[derive(Default)]
struct Data {
    projects: Vec<Project>,
    clients: Vec<Client>,
    tags: Vec<Tag>,
    /// Tracked milliseconds per project.
    spent: HashMap<Id, Millis>,
    /// Time entries per tag.
    tag_entries: HashMap<Id, i64>,
    /// Tasks of the open project and their tracked milliseconds.
    tasks: Vec<Task>,
    task_spent: HashMap<Id, Millis>,
}

pub struct CatalogController {
    ctx: Ctx,
    hub: Rc<TimerHub>,
    ui: slint::Weak<MainWindow>,
    data: RefCell<Data>,
    /// The project whose page is open.
    open: RefCell<Option<Id>>,
    projects: Rc<VecModel<ProjectRowView>>,
    tasks: Rc<VecModel<TaskRowView>>,
    clients: Rc<VecModel<ClientRowView>>,
    tags: Rc<VecModel<TagRowView>>,
    client_options: Rc<VecModel<ProjectOption>>,
}

/// Wires `CatalogBridge` of the main window and loads the catalog.
pub fn install(ui: &MainWindow, ctx: &Ctx, hub: &Rc<TimerHub>) -> Rc<CatalogController> {
    let ctl = Rc::new(CatalogController {
        ctx: ctx.clone(),
        hub: hub.clone(),
        ui: ui.as_weak(),
        data: RefCell::default(),
        open: RefCell::new(None),
        projects: Rc::default(),
        tasks: Rc::default(),
        clients: Rc::default(),
        tags: Rc::default(),
        client_options: Rc::default(),
    });
    let b = ui.global::<CatalogBridge>();
    b.set_projects(ModelRc::from(ctl.projects.clone()));
    b.set_tasks(ModelRc::from(ctl.tasks.clone()));
    b.set_clients(ModelRc::from(ctl.clients.clone()));
    b.set_tags(ModelRc::from(ctl.tags.clone()));
    b.set_client_options(ModelRc::from(ctl.client_options.clone()));
    b.set_palette(model(PALETTE.iter().map(|c| (*c).into()).collect()));

    let c = ctl.clone();
    b.on_refresh(move || c.reload());
    let c = ctl.clone();
    b.on_filter_changed(move || c.show());
    b.on_hex_color(|s| hex_color(s.trim()));

    let c = ctl.clone();
    b.on_open_project(move |id| c.open_project(&id));
    let c = ctl.clone();
    b.on_new_project(move || c.new_project());
    let c = ctl.clone();
    b.on_edit_project(move |id| c.edit_project(&id));
    let c = ctl.clone();
    b.on_save_project(move |d| c.save_project(&d));
    let c = ctl.clone();
    b.on_set_project_archived(move |id, archived| c.set_project_archived(&id, archived));
    let c = ctl.clone();
    b.on_delete_project(move |id| c.delete_project(&id));

    let c = ctl.clone();
    b.on_create_task(move |name| c.create_task(&name));
    let c = ctl.clone();
    b.on_edit_task(move |id| c.edit_task(&id));
    let c = ctl.clone();
    b.on_save_task(move |d| c.save_task(&d));
    let c = ctl.clone();
    b.on_toggle_task_done(move |id| c.toggle_task_done(&id));
    let c = ctl.clone();
    b.on_start_task(move |id| c.start_task(&id));
    let c = ctl.clone();
    b.on_delete_task(move |id| c.delete_task(&id));

    let c = ctl.clone();
    b.on_create_client(move |name| c.create_client(&name));
    let c = ctl.clone();
    b.on_rename_client(move |id, name| c.rename_client(&id, &name));
    let c = ctl.clone();
    b.on_set_client_archived(move |id, archived| c.set_client_archived(&id, archived));
    let c = ctl.clone();
    b.on_delete_client(move |id| c.delete_client(&id));

    let c = ctl.clone();
    b.on_create_tag(move |name| c.create_tag(&name));
    let c = ctl.clone();
    b.on_rename_tag(move |id, name| c.rename_tag(&id, &name));
    let c = ctl.clone();
    b.on_delete_tag(move |id| c.delete_tag(&id));

    // Tracked times change with the entries.
    let me = Rc::downgrade(&ctl);
    let events = ctx.borrow().events.clone();
    events.on_entries_changed(move || {
        if let Some(c) = me.upgrade() {
            c.reload();
        }
    });
    ctl.reload();
    ctl
}

impl CatalogController {
    /// Loads the catalog from the database and shows it.
    fn reload(&self) {
        let open = *self.open.borrow();
        let res = self.with_ctx(|c, now| load(c, open, now));
        match res {
            Ok(data) => {
                self.data.replace(data);
                self.show();
            }
            Err(e) => self.report("load", &e, |tr| error_message(tr, &e)),
        }
    }

    /// Shows the loaded catalog with the current filters.
    fn show(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let (hour, minute) = {
            let tr = ui.global::<Tr>();
            (tr.get_hour_suffix(), tr.get_minute_suffix())
        };
        let prefs = prefs_from(&self.ctx.borrow().settings, &hour, &minute);
        let b = ui.global::<CatalogBridge>();
        let d = self.data.borrow();
        let client = |p: &Project| {
            p.client_id
                .and_then(|id| d.clients.iter().find(|c| c.id == id))
        };
        let spent = |p: &Project| d.spent.get(&p.id).copied().unwrap_or(0);

        let (query, archived) = (b.get_search(), b.get_show_archived());
        let rows = visible_projects(&d.projects, &d.clients, &query, archived)
            .into_iter()
            .map(|p| project_row(p, client(p), spent(p), &prefs))
            .collect();
        sync_vec(&self.projects, rows);
        b.set_project_total(d.projects.len() as i32);

        let rows = visible_clients(&d.clients, b.get_show_archived_clients())
            .into_iter()
            .map(|c| {
                let n = d
                    .projects
                    .iter()
                    .filter(|p| p.client_id == Some(c.id))
                    .count();
                client_row(c, n)
            })
            .collect();
        sync_vec(&self.clients, rows);
        b.set_client_total(d.clients.len() as i32);

        let mut tags: Vec<&Tag> = d.tags.iter().collect();
        tags.sort_by_key(|t| t.name.to_lowercase());
        let rows = tags
            .into_iter()
            .map(|t| tag_row(t, d.tag_entries.get(&t.id).copied().unwrap_or(0)))
            .collect();
        sync_vec(&self.tags, rows);

        let options = visible_clients(&d.clients, false)
            .into_iter()
            .map(|c| ProjectOption {
                id: c.id.to_string().into(),
                label: c.name.as_str().into(),
                ..ProjectOption::default()
            })
            .collect();
        sync_vec(&self.client_options, options);

        let open = d
            .projects
            .iter()
            .find(|p| Some(p.id) == *self.open.borrow());
        if let Some(p) = open {
            b.set_project(project_row(p, client(p), spent(p), &prefs));
        }
        let rows = sorted_tasks(&d.tasks)
            .into_iter()
            .map(|t| task_row(t, d.task_spent.get(&t.id).copied().unwrap_or(0), &prefs))
            .collect();
        sync_vec(&self.tasks, rows);
    }

    // ---- projects ----

    fn open_project(&self, id: &str) {
        self.open.replace(parse_id(id));
        self.reload();
    }

    fn new_project(&self) {
        let n = self.data.borrow().projects.len();
        self.open_project_dialog(ProjectDialogData {
            color: palette_color(n).into(),
            ..ProjectDialogData::default()
        });
    }

    fn edit_project(&self, id: &str) {
        let d = self.data.borrow();
        let Some(p) = parse_id(id).and_then(|id| d.projects.iter().find(|p| p.id == id)) else {
            drop(d);
            return self.reload();
        };
        let client = p
            .client_id
            .and_then(|id| d.clients.iter().find(|c| c.id == id));
        let number = self.ctx.borrow().settings.format.number;
        let data = ProjectDialogData {
            id: p.id.to_string().into(),
            name: p.name.as_str().into(),
            color: p.color.as_str().into(),
            client_id: client.map_or(String::new(), |c| c.id.to_string()).into(),
            client_label: client.map_or("", |c| c.name.as_str()).into(),
            billable: p.is_billable,
            rate: p
                .billable_rate
                .map_or(String::new(), |r| money_text(r, number))
                .into(),
            estimate: p.estimated_time.map_or(String::new(), estimate_text).into(),
        };
        drop(d);
        self.open_project_dialog(data);
    }

    fn open_project_dialog(&self, data: ProjectDialogData) {
        if let Some(ui) = self.ui.upgrade() {
            let b = ui.global::<CatalogBridge>();
            b.set_project_dialog(data);
            b.set_project_dialog_open(true);
        }
    }

    /// Saves the project dialog; "" on success, else the message for the dialog.
    fn save_project(&self, d: &ProjectDialogData) -> SharedString {
        let number = self.ctx.borrow().settings.format.number;
        let Ok(rate) = parse_rate(&d.rate, number) else {
            return self.tr(|tr| tr.get_err_invalid_rate());
        };
        let Ok(estimate) = parse_estimate(&d.estimate) else {
            return self.tr(|tr| tr.get_err_invalid_estimate());
        };
        let (name, color) = (d.name.to_string(), d.color.trim().to_owned());
        let client_id = parse_id(&d.client_id);
        let res = match parse_id(&d.id) {
            None => self.with_ctx(|c, _| {
                c.catalog
                    .create_project(
                        &mut c.db,
                        NewProject {
                            name,
                            color,
                            client_id,
                            is_billable: d.billable,
                            billable_rate: rate,
                            estimated_time: estimate,
                        },
                    )
                    .map(drop)
            }),
            Some(id) => {
                let stored = self
                    .data
                    .borrow()
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .cloned();
                match stored {
                    None => Err(ServiceError::NotFound),
                    Some(p) => self.with_ctx(|c, _| {
                        let p = Project {
                            name,
                            color,
                            client_id,
                            is_billable: d.billable,
                            billable_rate: rate,
                            estimated_time: estimate,
                            ..p
                        };
                        c.catalog.update_project(&mut c.db, &p).map(drop)
                    }),
                }
            }
        };
        match res {
            Ok(()) => {
                self.changed();
                SharedString::new()
            }
            Err(e) => {
                log::warn!("catalog save project failed: {}", error_code(&e));
                self.tr(|tr| error_message(tr, &e))
            }
        }
    }

    fn set_project_archived(&self, id: &str, archived: bool) {
        let res = self.with_id(id, |c, id| {
            c.catalog
                .set_project_archived(&mut c.db, id, archived)
                .map(drop)
        });
        self.after("archive project", res);
    }

    fn delete_project(&self, id: &str) {
        let res = self.with_id(id, |c, id| c.catalog.delete_project(&mut c.db, id));
        self.after("delete project", res);
    }

    // ---- tasks of the open project ----

    fn create_task(&self, name: &str) -> SharedString {
        let Some(project) = *self.open.borrow() else {
            return SharedString::new();
        };
        let res = self.with_ctx(|c, _| c.catalog.create_task(&mut c.db, project, name, None));
        self.created("create task", res.map(|t| t.id))
    }

    fn find_task(&self, id: &str) -> Option<Task> {
        let id = parse_id(id)?;
        self.data
            .borrow()
            .tasks
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    fn edit_task(&self, id: &str) {
        let Some(t) = self.find_task(id) else {
            return self.reload();
        };
        if let Some(ui) = self.ui.upgrade() {
            let b = ui.global::<CatalogBridge>();
            b.set_task_dialog(TaskDialogData {
                id: t.id.to_string().into(),
                name: t.name.as_str().into(),
                estimate: t.estimated_time.map_or(String::new(), estimate_text).into(),
            });
            b.set_task_dialog_open(true);
        }
    }

    /// Saves the task dialog; "" on success, else the message for the dialog.
    fn save_task(&self, d: &TaskDialogData) -> SharedString {
        let Ok(estimate) = parse_estimate(&d.estimate) else {
            return self.tr(|tr| tr.get_err_invalid_estimate());
        };
        let Some(task) = self.find_task(&d.id) else {
            self.reload();
            return self.tr(|tr| tr.get_err_generic());
        };
        let task = Task {
            name: d.name.to_string(),
            estimated_time: estimate,
            ..task
        };
        match self.with_ctx(|c, _| c.catalog.update_task(&mut c.db, &task)) {
            Ok(_) => {
                self.changed();
                SharedString::new()
            }
            Err(e) => {
                log::warn!("catalog save task failed: {}", error_code(&e));
                self.tr(|tr| error_message(tr, &e))
            }
        }
    }

    fn toggle_task_done(&self, id: &str) {
        let Some(t) = self.find_task(id) else {
            return self.reload();
        };
        let done = t.done_at.is_none();
        let res = self.with_ctx(|c, _| c.catalog.set_task_done(&mut c.db, t.id, done).map(drop));
        self.after("task done", res);
    }

    /// Starts the timer with the task and its project (ruling R23).
    fn start_task(&self, id: &str) {
        let Some(t) = self.find_task(id).filter(|t| t.done_at.is_none()) else {
            return;
        };
        let billable = {
            let d = self.data.borrow();
            default_billable(d.projects.iter().find(|p| p.id == t.project_id))
        };
        self.hub.start_with(EntryTemplate {
            project_id: Some(t.project_id),
            task_id: Some(t.id),
            billable,
            ..EntryTemplate::default()
        });
    }

    fn delete_task(&self, id: &str) {
        let res = self.with_id(id, |c, id| c.catalog.delete_task(&mut c.db, id));
        match res {
            Ok(()) => self.changed(),
            Err(e) => self.report("delete task", &e, |tr| {
                // A task has no archive: the advice differs from the project's.
                if error_text(&e) == ErrorText::InUseEntries {
                    tr.get_err_in_use_task_entries()
                } else {
                    error_message(tr, &e)
                }
            }),
        }
    }

    // ---- clients ----

    fn create_client(&self, name: &str) -> SharedString {
        let res = self.with_ctx(|c, _| c.catalog.create_client(&mut c.db, name));
        self.created("create client", res.map(|c| c.id))
    }

    fn rename_client(&self, id: &str, name: &str) -> bool {
        let res = self.with_id(id, |c, id| {
            c.catalog.rename_client(&mut c.db, id, name).map(drop)
        });
        self.after("rename client", res)
    }

    fn set_client_archived(&self, id: &str, archived: bool) {
        let res = self.with_id(id, |c, id| {
            c.catalog
                .set_client_archived(&mut c.db, id, archived)
                .map(drop)
        });
        self.after("archive client", res);
    }

    fn delete_client(&self, id: &str) {
        let res = self.with_id(id, |c, id| c.catalog.delete_client(&mut c.db, id));
        self.after("delete client", res);
    }

    // ---- tags ----

    fn create_tag(&self, name: &str) -> SharedString {
        let res = self.with_ctx(|c, _| c.catalog.create_tag(&mut c.db, name));
        self.created("create tag", res.map(|t| t.id))
    }

    fn rename_tag(&self, id: &str, name: &str) -> bool {
        let res = self.with_id(id, |c, id| {
            c.catalog.rename_tag(&mut c.db, id, name).map(drop)
        });
        self.after("rename tag", res)
    }

    fn delete_tag(&self, id: &str) {
        let res = self.with_id(id, |c, id| c.catalog.delete_tag(&mut c.db, id));
        self.after("delete tag", res);
    }

    // ---- shared ----

    /// After a successful change: reload, then the timer pickers and the other lists show
    /// the stored state (R23).
    fn changed(&self) {
        self.reload();
        self.hub.refresh_catalog();
        let events = self.ctx.borrow().events.clone();
        events.emit_entries_changed();
    }

    /// `true` after a successful change, else the error has been shown.
    fn after(&self, op: &str, res: Result<()>) -> bool {
        match res {
            Ok(()) => {
                self.changed();
                true
            }
            Err(e) => {
                self.report(op, &e, |tr| error_message(tr, &e));
                false
            }
        }
    }

    /// The new id after a change, or "" after reporting the error.
    fn created(&self, op: &str, res: Result<Id>) -> SharedString {
        match res {
            Ok(id) => {
                self.changed();
                id.to_string().into()
            }
            Err(e) => {
                self.report(op, &e, |tr| error_message(tr, &e));
                SharedString::new()
            }
        }
    }

    /// Logs (code only), shows the message as a toast and re-syncs with the stored state.
    fn report(&self, op: &str, e: &ServiceError, text: impl FnOnce(&Tr<'_>) -> SharedString) {
        log::warn!("catalog {op} failed: {}", error_code(e));
        if let Some(ui) = self.ui.upgrade() {
            let text = text(&ui.global::<Tr>());
            ui.invoke_show_toast(text, ToastKind::Error);
        }
        if op != "load" {
            self.reload();
        }
    }

    fn with_id(&self, id: &str, f: impl FnOnce(&mut AppContext, Id) -> Result<()>) -> Result<()> {
        let id = parse_id(id).ok_or(ServiceError::NotFound)?;
        self.with_ctx(|c, _| f(c, id))
    }

    /// Runs `f` with the context and the current time (borrow ends on return).
    fn with_ctx<T>(&self, f: impl FnOnce(&mut AppContext, Millis) -> Result<T>) -> Result<T> {
        let mut c = self.ctx.borrow_mut();
        let now = c.timer.clock().now();
        f(&mut c, now)
    }

    fn tr(&self, f: impl FnOnce(&Tr<'_>) -> SharedString) -> SharedString {
        self.ui
            .upgrade()
            .map(|ui| f(&ui.global::<Tr>()))
            .unwrap_or_default()
    }
}

/// Reads the whole catalog, the tasks of `open` and the tracked times.
fn load(c: &AppContext, open: Option<Id>, now: Millis) -> Result<Data> {
    let conn = c.db.conn();
    let projects = c.catalog.list_projects(&c.db)?;
    let tags = c.catalog.list_tags(&c.db)?;
    let mut d = Data {
        clients: c.catalog.list_clients(&c.db)?,
        ..Data::default()
    };
    for p in &projects {
        d.spent
            .insert(p.id, entries::spent_ms_for_project(conn, p.id, now)?);
    }
    for t in &tags {
        d.tag_entries
            .insert(t.id, catalog::count_entries_for_tag(conn, t.id)?);
    }
    if let Some(id) = open.filter(|id| projects.iter().any(|p| p.id == *id)) {
        d.tasks = c.catalog.list_tasks(&c.db, id)?;
        for t in &d.tasks {
            d.task_spent
                .insert(t.id, entries::spent_ms_for_task(conn, t.id, now)?);
        }
    }
    d.projects = projects;
    d.tags = tags;
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_reads_catalog_with_tracked_times_and_tag_counts() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = AppContext::open(dir.path().join("tt.db")).unwrap();
        let c = &mut c;
        let client = c.catalog.create_client(&mut c.db, "Acme").unwrap();
        let project = c
            .catalog
            .create_project(
                &mut c.db,
                NewProject {
                    name: "Site".into(),
                    color: "#aabbcc".into(),
                    client_id: Some(client.id),
                    is_billable: false,
                    billable_rate: None,
                    estimated_time: None,
                },
            )
            .unwrap();
        let task = c
            .catalog
            .create_task(&mut c.db, project.id, "Reports", None)
            .unwrap();
        let other = c
            .catalog
            .create_project(
                &mut c.db,
                NewProject {
                    name: "Other".into(),
                    color: "#aabbcc".into(),
                    client_id: None,
                    is_billable: false,
                    billable_rate: None,
                    estimated_time: None,
                },
            )
            .unwrap();
        let tag = c.catalog.create_tag(&mut c.db, "design").unwrap();
        let t = EntryTemplate {
            project_id: Some(project.id),
            task_id: Some(task.id),
            tag_ids: vec![tag.id],
            ..EntryTemplate::default()
        };
        let tz = c.settings.format.tz;
        c.entries
            .create_manual(&mut c.db, t, 0, 3_600_000, tz, false)
            .unwrap();

        let d = load(c, Some(project.id), 10_000_000).unwrap();
        assert_eq!(d.projects.len(), 2);
        assert_eq!(d.clients.len(), 1);
        assert_eq!(d.spent[&project.id], 3_600_000);
        assert_eq!(d.spent[&other.id], 0);
        assert_eq!(d.tag_entries[&tag.id], 1);
        assert_eq!(d.tasks.len(), 1);
        assert_eq!(d.task_spent[&task.id], 3_600_000);

        // Without an open project no tasks are loaded.
        assert!(load(c, None, 10_000_000).unwrap().tasks.is_empty());
    }
}
