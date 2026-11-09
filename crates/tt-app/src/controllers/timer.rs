//! Timer bar and mini timer: start / pause / resume / stop, edits of the current entry
//! (or of the draft while stopped), the 1 s tick, and the project/task/tag options.
//!
//! [`TimerHub`] owns the state and pushes every update to all attached windows (each
//! window has its own `TimerBridge` global): the main window via [`install`], the mini
//! timer via [`TimerHub::attach`]. Closed windows are dropped on the next update.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, Global, Model, SharedString, Timer, TimerMode};
use tt_domain::{EntryTemplate, Id, SessionState, Tz};
use tt_services::{Clock, NewProject, Result, ServiceError, TimerStatus, default_billable};

use crate::app::{AppContext, Ctx};
use crate::controllers::{error_message, pickers::model};
use crate::vm::error::error_code;
use crate::vm::format::prefs_from;
use crate::vm::pickers::{Catalog, palette_color, parse_id, project_options, tag_options};
use crate::vm::timer::{active_template, timer_view};
use crate::{MainWindow, TimerBridge, TimerView, ToastKind, Tr};

/// Runs a function with one window's `TimerBridge`; false once that window is gone.
type Sink = Rc<dyn Fn(&dyn Fn(&TimerBridge<'_>)) -> bool>;

/// A closure sink without a window (the tray).
type ViewSink = Rc<dyn Fn(&TimerView)>;

type Reporter = Rc<dyn Fn(&ServiceError)>;

pub struct TimerHub {
    ctx: Ctx,
    me: Weak<TimerHub>,
    /// Attributes for the next Start while no session is active.
    draft: RefCell<EntryTemplate>,
    catalog: RefCell<Catalog>,
    windows: RefCell<Vec<Sink>>,
    /// Closure sinks without a window (the tray): get every view pushed to the windows.
    view_sinks: RefCell<Vec<ViewSink>>,
    report: Reporter,
    /// Localized (hour, minute) duration suffixes.
    suffixes: Box<dyn Fn() -> (SharedString, SharedString)>,
    /// Runs only while the session is running.
    ticker: Timer,
}

/// Creates the hub for the main window (errors become toasts there), catches up the
/// midnight rollover and shows the current state.
pub fn install(ui: &MainWindow, ctx: &Ctx) -> Rc<TimerHub> {
    let toast_ui = ui.as_weak();
    let tr_ui = ui.as_weak();
    let hub = TimerHub::new(
        ctx.clone(),
        Rc::new(move |e| {
            if let Some(ui) = toast_ui.upgrade() {
                let text = error_message(&ui.global::<Tr>(), e);
                ui.invoke_show_toast(text, ToastKind::Error);
            }
        }),
        Box::new(move || {
            tr_ui
                .upgrade()
                .map(|ui| {
                    let tr = ui.global::<Tr>();
                    (tr.get_hour_suffix(), tr.get_minute_suffix())
                })
                .unwrap_or_default()
        }),
    );
    hub.reload_catalog();
    hub.rollover();
    hub.attach(ui);

    // Formats, language and time zone change what the windows show.
    let me = Rc::downgrade(&hub);
    let events = ctx.borrow().events.clone();
    events.on_settings_changed(move || {
        if let Some(h) = me.upgrade() {
            h.refresh();
        }
    });
    hub
}

impl TimerHub {
    pub(crate) fn new(
        ctx: Ctx,
        report: Reporter,
        suffixes: Box<dyn Fn() -> (SharedString, SharedString)>,
    ) -> Rc<Self> {
        Rc::new_cyclic(|me| TimerHub {
            ctx,
            me: me.clone(),
            draft: RefCell::default(),
            catalog: RefCell::default(),
            windows: RefCell::default(),
            view_sinks: RefCell::default(),
            report,
            suffixes,
            ticker: Timer::default(),
        })
    }

    /// Connects the `TimerBridge` of `ui` and shows the current state in it.
    pub fn attach<C>(self: &Rc<Self>, ui: &C)
    where
        C: ComponentHandle + 'static,
        for<'a> TimerBridge<'a>: Global<'a, C>,
    {
        let b = ui.global::<TimerBridge>();
        let hub = |f: fn(&TimerHub)| {
            let h = self.clone();
            move || f(&h)
        };
        b.on_start(hub(TimerHub::start));
        b.on_pause(hub(TimerHub::pause));
        b.on_resume(hub(TimerHub::resume));
        b.on_stop(hub(TimerHub::stop));
        let h = self.clone();
        b.on_set_description(move |text| h.edit(|t| t.description = text.to_string()));
        let h = self.clone();
        b.on_set_project(move |project, task| h.set_project(&project, &task));
        let h = self.clone();
        b.on_set_tags(move |ids| {
            let ids: Vec<Id> = ids.iter().filter_map(|s| parse_id(&s)).collect();
            h.edit(|t| t.tag_ids = ids);
        });
        let h = self.clone();
        b.on_set_billable(move |on| h.edit(|t| t.billable = on));
        let h = self.clone();
        b.on_create_project(move |name| h.create_project(&name));
        let h = self.clone();
        b.on_create_task(move |project, name| h.create_task(&project, &name));
        let h = self.clone();
        b.on_create_tag(move |name| h.create_tag(&name));

        let h = self.clone();
        b.on_start_recent(move || h.start_recent());

        let weak = ui.as_weak();
        self.windows
            .borrow_mut()
            .push(Rc::new(move |f: &dyn Fn(&TimerBridge<'_>)| {
                weak.upgrade()
                    .map(|ui| f(&ui.global::<TimerBridge>()))
                    .is_some()
            }));
        self.push_all();
    }

    /// `f` gets the current view now and after every update (including the 1 s tick).
    pub fn on_view(&self, f: impl Fn(&TimerView) + 'static) {
        self.view_sinks.borrow_mut().push(Rc::new(f));
        self.push_view();
    }

    /// Reloads projects, tasks and tags (after catalog changes) and updates the windows.
    pub fn refresh_catalog(&self) {
        self.reload_catalog();
        self.push_all();
    }

    /// Shows the stored timer state again (after entries were changed elsewhere).
    pub fn refresh(&self) {
        self.push_all();
    }

    /// After the database was replaced (a restore): forgets the draft, which may name rows
    /// that are gone, reloads the options and shows the stored timer state.
    pub fn reset(&self) {
        self.draft.replace(EntryTemplate::default());
        self.refresh_catalog();
    }

    /// Runs `f` with the loaded projects, tasks and tags.
    pub fn with_catalog<R>(&self, f: impl FnOnce(&Catalog) -> R) -> R {
        f(&self.catalog.borrow())
    }

    fn start(&self) {
        let draft = self.draft.borrow().clone();
        if self
            .run("start", |c, tz| c.timer.start(&mut c.db, draft, tz))
            .is_some()
        {
            self.draft.replace(EntryTemplate::default());
            self.changed();
        }
    }

    /// Starts a session with `t` (the project page's task ▶), replacing the active one.
    pub fn start_with(&self, t: EntryTemplate) {
        if self
            .run("start", |c, tz| c.timer.start(&mut c.db, t, tz))
            .is_some()
        {
            self.draft.replace(EntryTemplate::default());
            self.changed();
        }
    }

    /// Starts a session with the attributes of the most recent entry (if there is one).
    fn start_recent(&self) {
        let res = self.with_ctx(|c, tz| match last_template(c)? {
            Some(t) => c.timer.start(&mut c.db, t, tz).map(Some),
            None => Ok(None),
        });
        match res {
            Ok(Some(_)) => self.changed(),
            Ok(None) => self.push_all(),
            Err(e) => self.fail("start recent", &e),
        }
    }

    pub fn pause(&self) {
        if self
            .run("pause", |c, tz| c.timer.pause(&mut c.db, tz))
            .is_some()
        {
            self.changed();
        }
    }

    pub fn resume(&self) {
        if self
            .run("resume", |c, tz| c.timer.resume(&mut c.db, tz))
            .is_some()
        {
            self.changed();
        }
    }

    pub fn stop(&self) {
        if self
            .run("stop", |c, tz| c.timer.stop(&mut c.db, tz))
            .is_some()
        {
            self.draft.replace(EntryTemplate::default());
            self.changed();
        }
    }

    fn set_project(&self, project: &str, task: &str) {
        let project_id = parse_id(project);
        let task_id = project_id.and(parse_id(task));
        let billable = default_billable(
            project_id
                .and_then(|id| self.catalog.borrow().project(id).cloned())
                .as_ref(),
        );
        self.edit(|t| {
            if t.project_id != project_id {
                t.billable = billable;
            }
            t.project_id = project_id;
            t.task_id = task_id;
        });
    }

    /// Applies `f` to the active session's template (saved at once) or to the draft.
    fn edit(&self, f: impl FnOnce(&mut EntryTemplate)) {
        let status = match self.status() {
            Ok(s) => s,
            Err(e) => return self.fail("status", &e),
        };
        let Some(mut t) = active_template(&status) else {
            f(&mut self.draft.borrow_mut());
            return self.push_all();
        };
        let before = t.clone();
        f(&mut t);
        if t != before
            && self
                .run("update", |c, tz| c.timer.update_template(&mut c.db, t, tz))
                .is_some()
        {
            self.changed();
        }
    }

    fn create_project(&self, name: &str) -> SharedString {
        let color = palette_color(self.catalog.borrow().projects.len()).to_owned();
        let res = self.with_ctx(|c, _| {
            c.catalog.create_project(
                &mut c.db,
                NewProject {
                    name: name.to_owned(),
                    color,
                    client_id: None,
                    is_billable: false,
                    billable_rate: None,
                    estimated_time: None,
                },
            )
        });
        self.created("create project", res.map(|p| p.id))
    }

    fn create_task(&self, project: &str, name: &str) -> SharedString {
        let res = match parse_id(project) {
            Some(pid) => self.with_ctx(|c, _| c.catalog.create_task(&mut c.db, pid, name, None)),
            None => Err(ServiceError::NotFound),
        };
        self.created("create task", res.map(|t| t.id))
    }

    fn create_tag(&self, name: &str) -> SharedString {
        let res = self.with_ctx(|c, _| c.catalog.create_tag(&mut c.db, name));
        self.created("create tag", res.map(|t| t.id))
    }

    /// The new id after reloading the options, or "" after reporting the error.
    fn created(&self, op: &'static str, res: Result<Id>) -> SharedString {
        match res {
            Ok(id) => {
                self.refresh_catalog();
                id.to_string().into()
            }
            Err(e) => {
                self.fail(op, &e);
                SharedString::new()
            }
        }
    }

    fn reload_catalog(&self) {
        let res = self.with_ctx(|c, _| {
            let projects = c.catalog.list_projects(&c.db)?;
            let mut tasks = Vec::new();
            for p in &projects {
                tasks.extend(c.catalog.list_tasks(&c.db, p.id)?);
            }
            let tags = c.catalog.list_tags(&c.db)?;
            Ok(Catalog {
                projects,
                tasks,
                tags,
            })
        });
        match res {
            Ok(catalog) => {
                self.catalog.replace(catalog);
            }
            Err(e) => self.fail("load catalog", &e),
        }
    }

    /// Splits a running entry at past midnights; tells the listeners if it did.
    fn rollover(&self) {
        match self.with_ctx(|c, tz| c.timer.rollover(&mut c.db, tz)) {
            Ok(true) => self.emit(),
            Ok(false) => {}
            // Not shown: the tick would repeat the toast every second.
            Err(e) => log::warn!("timer rollover failed: {}", error_code(&e)),
        }
    }

    fn tick(&self) {
        self.rollover();
        self.push_view();
    }

    fn status(&self) -> Result<TimerStatus> {
        self.with_ctx(|c, tz| c.timer.status(&c.db, tz))
    }

    /// Runs `f` with the context and the configured time zone (borrow ends on return).
    fn with_ctx<T>(&self, f: impl FnOnce(&mut AppContext, Tz) -> Result<T>) -> Result<T> {
        let mut c = self.ctx.borrow_mut();
        let tz = c.settings.format.tz;
        f(&mut c, tz)
    }

    fn run(
        &self,
        op: &'static str,
        f: impl FnOnce(&mut AppContext, Tz) -> Result<TimerStatus>,
    ) -> Option<TimerStatus> {
        self.with_ctx(f).map_err(|e| self.fail(op, &e)).ok()
    }

    /// Logs (code only), shows the error and re-syncs the windows with the stored state.
    fn fail(&self, op: &str, e: &ServiceError) {
        log::warn!("timer {op} failed: {}", error_code(e));
        (self.report)(e);
        self.push_all();
    }

    /// After a successful timer action: update the windows, then tell the listeners.
    fn changed(&self) {
        self.push_all();
        self.emit();
    }

    fn emit(&self) {
        let events = self.ctx.borrow().events.clone();
        events.emit_entries_changed();
    }

    fn view(&self) -> Result<(TimerView, TimerStatus)> {
        let (hour, minute) = (self.suffixes)();
        let (status, prefs, now) = {
            let c = self.ctx.borrow();
            let prefs = prefs_from(&c.settings, &hour, &minute);
            (
                c.timer.status(&c.db, prefs.tz)?,
                prefs,
                c.timer.clock().now(),
            )
        };
        let view = timer_view(
            &status,
            &self.draft.borrow(),
            &self.catalog.borrow(),
            &prefs,
            now,
        );
        Ok((view, status))
    }

    /// Runs `f` with the `TimerBridge` of every attached window and forgets closed windows.
    fn each_window(&self, f: &dyn Fn(&TimerBridge<'_>)) {
        let sinks = self.windows.borrow().clone();
        let dead: Vec<Sink> = sinks.into_iter().filter(|s| !s(f)).collect();
        if !dead.is_empty() {
            self.windows
                .borrow_mut()
                .retain(|s| !dead.iter().any(|d| Rc::ptr_eq(s, d)));
        }
    }

    fn push_view(&self) {
        let (view, status) = match self.view() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("timer status failed: {}", error_code(&e));
                return;
            }
        };
        self.each_window(&|b| b.set_view(view.clone()));
        let sinks = self.view_sinks.borrow().clone();
        for f in sinks {
            f(&view);
        }
        let running = status
            .session
            .is_some_and(|s| s.state == SessionState::Running);
        self.set_ticking(running);
    }

    fn push_all(&self) {
        let selected = self
            .status()
            .ok()
            .and_then(|s| active_template(&s))
            .unwrap_or_else(|| self.draft.borrow().clone())
            .tag_ids;
        let (projects, tags) = {
            let c = self.catalog.borrow();
            (project_options(&c), tag_options(&c, &selected))
        };
        let recent = self
            .with_ctx(|c, _| last_template(c))
            .map(|t| t.is_some())
            .unwrap_or_else(|e| {
                log::warn!("timer recent entry failed: {}", error_code(&e));
                false
            });
        self.each_window(&|b| {
            b.set_projects(model(projects.clone()));
            b.set_tags(model(tags.clone()));
            b.set_can_start_recent(recent);
        });
        self.push_view();
    }

    fn set_ticking(&self, on: bool) {
        if on && !self.ticker.running() {
            let me = self.me.clone();
            self.ticker
                .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                    if let Some(hub) = me.upgrade() {
                        hub.tick();
                    }
                });
        } else if !on && self.ticker.running() {
            self.ticker.stop();
        }
    }
}

/// Attributes of the most recent entry (the mini timer's "start the last task").
fn last_template(c: &AppContext) -> Result<Option<EntryTemplate>> {
    let latest = tt_storage::entries::recent_entries(c.db.conn(), 1)?;
    Ok(latest.first().map(EntryTemplate::from_entry))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn hub(dir: &tempfile::TempDir) -> Rc<TimerHub> {
        let ctx = AppContext::open(dir.path().join("tt.db")).unwrap();
        TimerHub::new(
            Rc::new(RefCell::new(ctx)),
            Rc::new(|_| {}),
            Box::new(Default::default),
        )
    }

    /// A window stand-in that counts updates; `alive` = whether its window still exists.
    fn fake(calls: &Rc<Cell<u32>>, alive: bool) -> Sink {
        let calls = calls.clone();
        Rc::new(move |_| {
            calls.set(calls.get() + 1);
            alive
        })
    }

    #[test]
    fn closed_windows_are_dropped_on_update() {
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(&dir);
        let (open, closed) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        hub.windows
            .borrow_mut()
            .extend([fake(&closed, false), fake(&open, true)]);

        hub.refresh();
        assert_eq!(hub.windows.borrow().len(), 1);
        let (open_calls, closed_calls) = (open.get(), closed.get());
        assert!(open_calls > 0);

        hub.refresh();
        assert!(
            open.get() > open_calls,
            "the open window still gets updates"
        );
        assert_eq!(
            closed.get(),
            closed_calls,
            "the closed one is not asked again"
        );
    }

    #[test]
    fn view_sinks_get_the_current_view_and_every_change() {
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(&dir);
        let states = Rc::new(RefCell::new(Vec::new()));
        let s = states.clone();
        hub.on_view(move |v| s.borrow_mut().push(v.state));
        assert_eq!(*states.borrow(), [0], "current view right away");

        hub.start();
        hub.pause();
        hub.resume();
        hub.stop();
        let mut seen = states.borrow().clone();
        seen.dedup();
        assert_eq!(seen, [0, 1, 2, 1, 0]);
    }

    #[test]
    fn last_template_is_the_latest_entry() {
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(&dir);
        assert_eq!(last_template(&hub.ctx.borrow()).unwrap(), None);

        let first = EntryTemplate {
            description: "first".into(),
            ..EntryTemplate::default()
        };
        let second = EntryTemplate {
            description: "second".into(),
            billable: true,
            ..EntryTemplate::default()
        };
        for t in [first, second.clone()] {
            let mut c = hub.ctx.borrow_mut();
            let c = &mut *c;
            let tz = c.settings.format.tz;
            c.timer.start(&mut c.db, t, tz).unwrap();
            c.timer.stop(&mut c.db, tz).unwrap();
        }
        assert_eq!(last_template(&hub.ctx.borrow()).unwrap(), Some(second));
    }
}
