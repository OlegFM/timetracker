//! Timer bar and mini timer: start / pause / resume / stop, edits of the current entry
//! (or of the draft while stopped), the 1 s tick, and the project/task/tag options.
//!
//! [`TimerHub`] owns the state and pushes every update to all attached windows (each
//! window has its own `TimerBridge` global): the main window via [`install`], the mini
//! timer later via [`TimerHub::attach`].

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
use crate::{MainWindow, ProjectOption, TagOption, TimerBridge, TimerView, ToastKind, Tr};

/// Pushes state into one window's `TimerBridge`.
struct Sink {
    view: Box<dyn Fn(&TimerView)>,
    options: OptionsSink,
}

type Reporter = Rc<dyn Fn(&ServiceError)>;
type OptionsSink = Box<dyn Fn(&[ProjectOption], &[TagOption])>;

pub struct TimerHub {
    ctx: Ctx,
    me: Weak<TimerHub>,
    /// Attributes for the next Start while no session is active.
    draft: RefCell<EntryTemplate>,
    catalog: RefCell<Catalog>,
    windows: RefCell<Vec<Rc<Sink>>>,
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
    hub
}

impl TimerHub {
    fn new(
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

        let (view_ui, options_ui) = (ui.as_weak(), ui.as_weak());
        self.windows.borrow_mut().push(Rc::new(Sink {
            view: Box::new(move |v| {
                if let Some(ui) = view_ui.upgrade() {
                    ui.global::<TimerBridge>().set_view(v.clone());
                }
            }),
            options: Box::new(move |projects, tags| {
                if let Some(ui) = options_ui.upgrade() {
                    let b = ui.global::<TimerBridge>();
                    b.set_projects(model(projects.to_vec()));
                    b.set_tags(model(tags.to_vec()));
                }
            }),
        }));
        self.push_all();
    }

    /// Reloads projects, tasks and tags (after catalog changes) and updates the windows.
    pub fn refresh_catalog(&self) {
        self.reload_catalog();
        self.push_all();
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

    fn pause(&self) {
        if self
            .run("pause", |c, tz| c.timer.pause(&mut c.db, tz))
            .is_some()
        {
            self.changed();
        }
    }

    fn resume(&self) {
        if self
            .run("resume", |c, tz| c.timer.resume(&mut c.db, tz))
            .is_some()
        {
            self.changed();
        }
    }

    fn stop(&self) {
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

    fn sinks(&self) -> Vec<Rc<Sink>> {
        self.windows.borrow().clone()
    }

    fn push_view(&self) {
        let (view, status) = match self.view() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("timer status failed: {}", error_code(&e));
                return;
            }
        };
        for s in self.sinks() {
            (s.view)(&view);
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
        for s in self.sinks() {
            (s.options)(&projects, &tags);
        }
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
