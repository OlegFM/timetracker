//! Dashboard screen: recent tasks, last 7 days, activity, this week and the project split.
//!
//! Same lazy pattern as the catalog controller: the data is reloaded when the screen is
//! shown while stale, and on entry changes only while the dashboard is the open page
//! (otherwise it is only marked stale). There are no timers: a running entry counts up to
//! the moment of the last reload.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use tt_domain::{EntryTemplate, Id, Millis};
use tt_services::{Clock, Dashboard, DashboardService, Result};

use crate::app::{AppContext, Ctx};
use crate::controllers::error_message;
use crate::controllers::sync_vec;
use crate::controllers::timer::TimerHub;
use crate::vm::charts::{Labels, dashboard_view};
use crate::vm::error::error_code;
use crate::vm::format::prefs_from;
use crate::vm::pickers::Catalog;
use crate::{
    BarView, DashboardBridge, HeatCell, MainWindow, PieSlice, ProjectTotalView, RecentView,
    ToastKind, Tr,
};

/// The page index of the dashboard in `MainWindow`.
const PAGE: i32 = 0;

pub struct DashboardController {
    ctx: Ctx,
    hub: Rc<TimerHub>,
    ui: slint::Weak<MainWindow>,
    /// Templates behind the recent tasks shown, in order.
    recent_templates: RefCell<Vec<EntryTemplate>>,
    recent: Rc<VecModel<RecentView>>,
    last7: Rc<VecModel<BarView>>,
    heatmap: Rc<VecModel<HeatCell>>,
    week: Rc<VecModel<BarView>>,
    week_projects: Rc<VecModel<ProjectTotalView>>,
    pie: Rc<VecModel<PieSlice>>,
    /// Number of database reloads (tests).
    reloads: Cell<u32>,
}

/// Wires `DashboardBridge` of the main window and loads the first data.
pub fn install(ui: &MainWindow, ctx: &Ctx, hub: &Rc<TimerHub>) -> Rc<DashboardController> {
    let ctl = Rc::new(DashboardController {
        ctx: ctx.clone(),
        hub: hub.clone(),
        ui: ui.as_weak(),
        recent_templates: RefCell::default(),
        recent: Rc::default(),
        last7: Rc::default(),
        heatmap: Rc::default(),
        week: Rc::default(),
        week_projects: Rc::default(),
        pie: Rc::default(),
        reloads: Cell::new(0),
    });
    let b = ui.global::<DashboardBridge>();
    b.set_recent(ModelRc::from(ctl.recent.clone()));
    b.set_last7(ModelRc::from(ctl.last7.clone()));
    b.set_heatmap(ModelRc::from(ctl.heatmap.clone()));
    b.set_week(ModelRc::from(ctl.week.clone()));
    b.set_week_projects(ModelRc::from(ctl.week_projects.clone()));
    b.set_pie(ModelRc::from(ctl.pie.clone()));

    let c = ctl.clone();
    b.on_refresh(move || c.reload());
    let c = ctl.clone();
    b.on_start_recent(move |index| c.start_recent(index));

    ctl.listen_to_entries();
    ctl.reload();
    ctl
}

impl DashboardController {
    /// Totals change with the entries (the formats with the settings): reload now if the
    /// dashboard is shown. Otherwise nothing: opening the page always reloads (the cards also
    /// depend on the current time).
    fn listen_to_entries(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        let events = self.ctx.borrow().events.clone();
        events.on_data_changed(move || {
            if let Some(c) = me.upgrade().filter(|c| c.page_visible()) {
                c.reload();
            }
        });
    }

    fn page_visible(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| ui.get_page() == PAGE)
    }

    /// Loads the dashboard from the database and shows it.
    fn reload(&self) {
        self.reloads.set(self.reloads.get() + 1);
        let res = {
            let c = self.ctx.borrow();
            let now = c.timer.clock().now();
            load(&c, now)
        };
        match res {
            Ok((dashboard, catalog)) => {
                self.recent_templates.replace(dashboard.recent.clone());
                self.show(&dashboard, &catalog);
            }
            Err(e) => {
                log::warn!("dashboard load failed: {}", error_code(&e));
                if let Some(ui) = self.ui.upgrade() {
                    let text = error_message(&ui.global::<Tr>(), &e);
                    ui.invoke_show_toast(text, ToastKind::Error);
                }
            }
        }
    }

    fn show(&self, dashboard: &Dashboard, catalog: &Catalog) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let tr = ui.global::<Tr>();
        let labels = Labels {
            weekdays: [
                tr.get_weekday_mon(),
                tr.get_weekday_tue(),
                tr.get_weekday_wed(),
                tr.get_weekday_thu(),
                tr.get_weekday_fri(),
                tr.get_weekday_sat(),
                tr.get_weekday_sun(),
            ]
            .map(|s| s.to_string()),
            no_project: tr.get_no_project().to_string(),
        };
        let prefs = prefs_from(
            &self.ctx.borrow().settings,
            &tr.get_hour_suffix(),
            &tr.get_minute_suffix(),
        );
        let v = dashboard_view(dashboard, catalog, &labels, &prefs);

        let b = ui.global::<DashboardBridge>();
        b.set_has_last7(v.last7.iter().any(|x| x.value > 0.0));
        b.set_has_heatmap(v.heatmap.iter().any(|x| x.level > 0));
        b.set_last7_total(v.last7_total.into());
        b.set_week_total(v.week_total.into());
        b.set_week_cost(v.week_cost.into());
        sync_vec(&self.recent, v.recent);
        sync_vec(&self.last7, v.last7);
        sync_vec(&self.heatmap, v.heatmap);
        sync_vec(&self.week, v.week);
        sync_vec(&self.week_projects, v.week_projects);
        sync_vec(&self.pie, v.pie);
    }

    /// Starts the timer with the recent task at `index`.
    fn start_recent(&self, index: i32) {
        let template = usize::try_from(index)
            .ok()
            .and_then(|i| self.recent_templates.borrow().get(i).cloned());
        match template {
            Some(t) => self.hub.start_with(t),
            // The list changed under the click: show the current one.
            None => self.reload(),
        }
    }
}

/// Reads the dashboard and the projects and tasks its recent tasks refer to.
fn load(c: &AppContext, now: Millis) -> Result<(Dashboard, Catalog)> {
    let dashboard = DashboardService::load(&c.db, &c.settings, now)?;
    let projects = c.catalog.list_projects(&c.db)?;
    let with_tasks: BTreeSet<Id> = dashboard
        .recent
        .iter()
        .filter(|t| t.task_id.is_some())
        .filter_map(|t| t.project_id)
        .collect();
    let mut tasks = Vec::new();
    for id in with_tasks {
        tasks.extend(c.catalog.list_tasks(&c.db, id)?);
    }
    let catalog = Catalog {
        projects,
        tasks,
        tags: Vec::new(),
    };
    Ok((dashboard, catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_domain::{TimeEntry, TimerSession};
    use tt_services::NewProject;

    fn ctx(dir: &tempfile::TempDir) -> Ctx {
        Rc::new(RefCell::new(
            AppContext::open(dir.path().join("tt.db")).unwrap(),
        ))
    }

    fn controller(ctx: &Ctx) -> Rc<DashboardController> {
        let hub = TimerHub::new(ctx.clone(), Rc::new(|_| {}), Box::new(Default::default));
        let ctl = Rc::new(DashboardController {
            ctx: ctx.clone(),
            hub,
            ui: slint::Weak::default(),
            recent_templates: RefCell::default(),
            recent: Rc::default(),
            last7: Rc::default(),
            heatmap: Rc::default(),
            week: Rc::default(),
            week_projects: Rc::default(),
            pie: Rc::default(),
            reloads: Cell::new(0),
        });
        ctl.listen_to_entries();
        ctl
    }

    #[test]
    fn load_reads_recent_tasks_with_their_project_and_task() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let mut c = ctx.borrow_mut();
        let c = &mut *c;
        let project = c
            .catalog
            .create_project(
                &mut c.db,
                NewProject {
                    name: "Site".into(),
                    color: "#aabbcc".into(),
                    client_id: None,
                    is_billable: false,
                    billable_rate: None,
                    estimated_time: None,
                },
            )
            .unwrap();
        let task = c
            .catalog
            .create_task(&mut c.db, project.id, "Login", None)
            .unwrap();
        let template = EntryTemplate {
            description: "fix".into(),
            project_id: Some(project.id),
            task_id: Some(task.id),
            ..EntryTemplate::default()
        };
        let now = c.timer.clock().now();
        let tz = c.settings.format.tz;
        c.entries
            .create_manual(
                &mut c.db,
                template.clone(),
                now - 7_200_000,
                now - 3_600_000,
                tz,
                false,
            )
            .unwrap();

        let (dash, catalog) = load(c, now).unwrap();
        assert_eq!(dash.recent, [template]);
        assert_eq!(catalog.projects.len(), 1);
        assert_eq!(catalog.task(task.id).unwrap().name, "Login");
    }

    #[test]
    fn opening_always_reloads_and_entry_events_only_reload_a_visible_page() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let ctl = controller(&ctx);

        // Opening the page twice with no events in between: two reloads (time moves on).
        ctl.reload();
        ctl.reload();
        assert_eq!(ctl.reloads.get(), 2);

        // No window = the dashboard is not the open page: events do not reload.
        let events = ctx.borrow().events.clone();
        events.emit_entries_changed();
        assert_eq!(ctl.reloads.get(), 2);
    }

    #[test]
    fn clicking_a_recent_task_starts_the_timer_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let ctl = controller(&ctx);
        {
            let mut c = ctx.borrow_mut();
            let c = &mut *c;
            let now = c.timer.clock().now();
            let tz = c.settings.format.tz;
            let t = EntryTemplate {
                description: "write docs".into(),
                ..EntryTemplate::default()
            };
            c.entries
                .create_manual(&mut c.db, t, now - 7_200_000, now - 3_600_000, tz, false)
                .unwrap();
        }
        ctl.reload();
        assert_eq!(ctl.recent_templates.borrow().len(), 1);

        // Out of range clicks do nothing but refresh.
        ctl.start_recent(5);
        ctl.start_recent(-1);
        assert!(running(&ctx).is_none());

        ctl.start_recent(0);
        let (entry, _session) = running(&ctx).expect("timer started");
        assert_eq!(entry.description, "write docs");
    }

    fn running(ctx: &Ctx) -> Option<(TimeEntry, TimerSession)> {
        let c = ctx.borrow();
        let entry = tt_storage::entries::running_entry(c.db.conn()).unwrap()?;
        let session = tt_storage::entries::active_session(c.db.conn()).unwrap()?;
        Some((entry, session))
    }
}
