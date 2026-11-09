//! Reports screen: filters, groupings, charts, table and CSV / XLSX export.
//!
//! The UI changes the filter properties of `ReportsBridge` and calls `run()`. The
//! controller reads them back as a [`ReportState`], normalizes it, saves it in the
//! settings (`report.last`), runs the report and writes state and results back. The data
//! is small, so every change reruns the report at once. Entry changes rerun it only while
//! the Reports page is open (opening the page always runs it).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::rc::Rc;

use chrono::NaiveDate;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tt_domain::{
    Client, FormatPrefs, GroupBy, GroupKey, Id, Millis, ReportResult, Tag, TimeEntry, local_date,
};
use tt_services::{
    Clock, ExportLabels, Lookup, ReportService, Result, ServiceError, export_entries_csv,
    export_report_csv, export_report_xlsx,
};
use tt_storage::settings;

use crate::app::{AppContext, Ctx};
use crate::controllers::pickers::model;
use crate::controllers::{error_message, sync_vec};
use crate::vm::error::error_code;
use crate::vm::format::prefs_from;
use crate::vm::pickers::{Catalog, parse_id};
use crate::vm::report::{
    GROUPS, Labels, Names, ReportState, SETTING_KEY, client_filter, group_value, project_filter,
    range_text, report_view, tag_filter, task_filter,
};
use crate::{
    BarView, MainWindow, PieSlice, ReportRowView, ReportsBridge, SelectLabels, SelectOption,
    ToastKind, Tr,
};

/// The page index of the reports in `MainWindow`.
const PAGE: i32 = 6;

/// What a save dialog exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Export {
    ReportCsv,
    EntriesCsv,
    Xlsx,
}

impl Export {
    fn extension(self) -> &'static str {
        match self {
            Self::ReportCsv | Self::EntriesCsv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

/// Projects, tasks, clients and tags as stored (archived projects and done tasks too:
/// they have history).
#[derive(Default)]
struct Data {
    catalog: Catalog,
    clients: Vec<Client>,
}

/// A report run: the normalized state, the period and the result.
struct Computed {
    state: ReportState,
    data: Data,
    today: NaiveDate,
    range: (NaiveDate, NaiveDate),
    result: ReportResult,
}

pub struct ReportsController {
    ctx: Ctx,
    ui: slint::Weak<MainWindow>,
    /// The state of the last run.
    state: RefCell<ReportState>,
    /// The JSON last written to the settings.
    saved: RefCell<String>,
    bars: Rc<VecModel<BarView>>,
    pie: Rc<VecModel<PieSlice>>,
    rows: Rc<VecModel<ReportRowView>>,
    /// Number of report runs (tests).
    runs: std::cell::Cell<u32>,
}

/// Wires `ReportsBridge` (and the `SelectLabels` helper) of the main window and loads the
/// saved report state into it.
pub fn install(ui: &MainWindow, ctx: &Ctx) -> Rc<ReportsController> {
    let (state, saved) = {
        let c = ctx.borrow();
        let saved = settings::get(c.db.conn(), SETTING_KEY).unwrap_or(None);
        (
            ReportState::parse(saved.as_deref()),
            saved.unwrap_or_default(),
        )
    };
    let ctl = Rc::new(ReportsController {
        ctx: ctx.clone(),
        ui: ui.as_weak(),
        state: RefCell::new(state),
        saved: RefCell::new(saved),
        bars: Rc::default(),
        pie: Rc::default(),
        rows: Rc::default(),
        runs: std::cell::Cell::new(0),
    });
    ui.global::<SelectLabels>().on_label(|options, value| {
        slint::Model::iter(&options)
            .find(|o| o.value == value)
            .map(|o| o.label)
            .unwrap_or_default()
    });
    let b = ui.global::<ReportsBridge>();
    b.set_bars(ModelRc::from(ctl.bars.clone()));
    b.set_pie(ModelRc::from(ctl.pie.clone()));
    b.set_rows(ModelRc::from(ctl.rows.clone()));
    ctl.push_state(ui, &ctl.state.borrow());

    let c = ctl.clone();
    b.on_run(move || c.run());
    let c = ctl.clone();
    b.on_export_csv(move || c.export(Export::ReportCsv));
    let c = ctl.clone();
    b.on_export_entries_csv(move || c.export(Export::EntriesCsv));
    let c = ctl.clone();
    b.on_export_xlsx(move || c.export(Export::Xlsx));

    ctl.listen_to_entries();
    ctl
}

impl ReportsController {
    fn listen_to_entries(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        let events = self.ctx.borrow().events.clone();
        events.on_data_changed(move || {
            if let Some(c) = me.upgrade().filter(|c| c.page_visible()) {
                c.run();
            }
        });
    }

    fn page_visible(&self) -> bool {
        self.ui.upgrade().is_some_and(|ui| ui.get_page() == PAGE)
    }

    /// Reads the filter properties, runs the report and shows it.
    fn run(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        self.runs.set(self.runs.get() + 1);
        let wanted = self.read_state(&ui);
        let res = {
            let c = self.ctx.borrow();
            let now = c.timer.clock().now();
            compute(&c, wanted, now)
        };
        match res {
            Ok(done) => {
                self.save(&done.state);
                self.show(&ui, &done);
                self.state.replace(done.state);
            }
            Err(e) => {
                log::warn!("report failed: {}", error_code(&e));
                ui.invoke_show_toast(error_message(&ui.global::<Tr>(), &e), ToastKind::Error);
            }
        }
    }

    /// The state the bridge properties describe, on top of the last one.
    fn read_state(&self, ui: &MainWindow) -> ReportState {
        let b = ui.global::<ReportsBridge>();
        let mut s = self.state.borrow().clone();
        let ids = |m: ModelRc<SharedString>| -> Vec<Id> {
            slint::Model::iter(&m)
                .filter_map(|x| parse_id(&x))
                .collect()
        };
        let was_preset = s.preset.is_some();
        let before = {
            let c = self.ctx.borrow();
            let f = &c.settings.format;
            s.range(local_date(c.timer.clock().now(), f.tz), f.week_start)
        };
        s.set_preset_value(b.get_preset());
        if was_preset && s.preset.is_none() {
            // Switching to the custom range starts from the period shown so far.
            (s.custom_from, s.custom_to) = crate::vm::report::custom_dates(before.0, before.1);
        } else {
            s.custom_from = b.get_custom_from().to_string();
            s.custom_to = b.get_custom_to().to_string();
        }
        s.project_ids = ids(b.get_project_ids());
        s.task_ids = ids(b.get_task_ids());
        s.client_ids = ids(b.get_client_ids());
        s.tag_ids = ids(b.get_tag_ids());
        s.tag_match = if b.get_tag_match_all() {
            tt_domain::TagMatch::All
        } else {
            tt_domain::TagMatch::Any
        };
        s.set_billable_value(b.get_billable());
        s.set_groups(b.get_group1(), b.get_group2());
        s.set_interval_value(b.get_interval());
        s.set_rounding_values(b.get_rounding_mode(), b.get_rounding_step());
        s.normalized()
    }

    /// Writes the state to the settings when it differs from what is stored.
    fn save(&self, state: &ReportState) {
        let json = state.to_json();
        if *self.saved.borrow() == json {
            return;
        }
        let res = {
            let c = self.ctx.borrow();
            settings::set(c.db.conn(), SETTING_KEY, &json)
        };
        match res {
            Ok(()) => {
                self.saved.replace(json);
            }
            Err(e) => log::warn!("report state not saved: {e}"),
        }
    }

    fn show(&self, ui: &MainWindow, done: &Computed) {
        let tr = ui.global::<Tr>();
        let (prefs, date) = {
            let c = self.ctx.borrow();
            (
                prefs_from(&c.settings, &tr.get_hour_suffix(), &tr.get_minute_suffix()),
                c.settings.format.date,
            )
        };
        let labels = labels(&tr);
        let names = Names {
            catalog: &done.data.catalog,
            clients: &done.data.clients,
            labels: &labels,
            date,
        };
        let s = &done.state;
        let v = report_view(&done.result, s.g1, s.g2, s.interval, &names, &prefs);

        self.push_state(ui, s);
        let b = ui.global::<ReportsBridge>();
        b.set_range_text(range_text(done.range.0, done.range.1, date).into());
        if s.preset.is_none() {
            let (from, to) = crate::vm::report::custom_dates(done.range.0, done.range.1);
            b.set_custom_from(from.into());
            b.set_custom_to(to.into());
        }
        b.set_projects(project_filter(&done.data.catalog, &s.project_ids));
        b.set_tasks(task_filter(&done.data.catalog, &s.task_ids));
        b.set_clients(client_filter(&done.data.clients, &s.client_ids));
        b.set_tags(tag_filter(&done.data.catalog, &s.tag_ids));
        b.set_group1_options(ModelRc::new(VecModel::from(group_options(&tr, None))));
        b.set_group2_options(ModelRc::new(VecModel::from(group_options(&tr, Some(s.g1)))));
        b.set_total(v.total.into());
        b.set_cost(v.cost.into());
        sync_vec(&self.bars, v.bars);
        sync_vec(&self.pie, v.pie);
        sync_vec(&self.rows, v.rows);
    }

    /// Writes the plain state (periods, ids, groupings) into the bridge.
    fn push_state(&self, ui: &MainWindow, s: &ReportState) {
        let b = ui.global::<ReportsBridge>();
        let ids = |v: &[Id]| model(v.iter().map(|i| i.to_string().into()).collect());
        b.set_preset(s.preset_value());
        b.set_custom_from(s.custom_from.as_str().into());
        b.set_custom_to(s.custom_to.as_str().into());
        b.set_project_ids(ids(&s.project_ids));
        b.set_task_ids(ids(&s.task_ids));
        b.set_client_ids(ids(&s.client_ids));
        b.set_tag_ids(ids(&s.tag_ids));
        b.set_tag_match_all(s.tag_match == tt_domain::TagMatch::All);
        b.set_billable(s.billable_value());
        b.set_group1(group_value(s.g1));
        b.set_group2(s.g2.map_or(-1, group_value));
        b.set_interval(s.interval_value());
        b.set_rounding_mode(s.rounding_mode_value());
        b.set_rounding_step(s.rounding_step_value());
    }

    /// Asks where to save, then writes the current report there.
    fn export(&self, kind: Export) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        // The report as it is now (the entries may have changed since the last run).
        let prepared = {
            let c = self.ctx.borrow();
            let now = c.timer.clock().now();
            compute(&c, self.state.borrow().clone(), now)
                .and_then(|done| entries_of(&c, &done, now).map(|e| (done, e, now)))
        };
        let (done, entries, now) = match prepared {
            Ok(p) => p,
            Err(e) => return self.fail(&ui, &e),
        };
        let tz = self.ctx.borrow().settings.format.tz;
        let ext = kind.extension();
        let file = format!(
            "timetracker-report-{}.{ext}",
            local_date(now, tz).format("%Y-%m-%d")
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter(ext.to_uppercase(), &[ext])
            .set_file_name(file)
            .save_file()
        else {
            return;
        };

        let tr = ui.global::<Tr>();
        let prefs = {
            let c = self.ctx.borrow();
            prefs_from(&c.settings, &tr.get_hour_suffix(), &tr.get_minute_suffix())
        };
        let data = ExportData {
            result: &done.result,
            entries: &entries,
            data: &done.data,
            g1: done.state.g1,
            g2: done.state.g2,
            labels: labels(&tr),
            export_labels: export_labels(&tr),
            prefs,
        };
        match write_export(kind, &path, &data) {
            Ok(()) => ui.invoke_show_toast(tr.get_exported(), ToastKind::Success),
            Err(e) => self.fail(&ui, &e),
        }
    }

    fn fail(&self, ui: &MainWindow, e: &ServiceError) {
        log::warn!("report export failed: {}", error_code(e));
        ui.invoke_show_toast(error_message(&ui.global::<Tr>(), e), ToastKind::Error);
    }
}

fn load_data(c: &AppContext) -> Result<Data> {
    let projects = c.catalog.list_projects(&c.db)?;
    let mut tasks = Vec::new();
    for p in &projects {
        tasks.extend(c.catalog.list_tasks(&c.db, p.id)?);
    }
    let tags: Vec<Tag> = c.catalog.list_tags(&c.db)?;
    Ok(Data {
        catalog: Catalog {
            projects,
            tasks,
            tags,
        },
        clients: c.catalog.list_clients(&c.db)?,
    })
}

/// Runs the report of `state` (filter ids that no longer exist are dropped).
fn compute(c: &AppContext, mut state: ReportState, now: Millis) -> Result<Computed> {
    let data = load_data(c)?;
    state.retain_existing(&data.catalog, &data.clients);
    let f = &c.settings.format;
    let today = local_date(now, f.tz);
    let filter = state.filter(today, f.week_start, f.tz);
    let result = ReportService::run(
        &c.db,
        &filter,
        state.g1,
        state.g2,
        state.interval,
        &c.settings,
        now,
    )?;
    Ok(Computed {
        range: state.range(today, f.week_start),
        state,
        data,
        today,
        result,
    })
}

/// The entries the report covers (for the entries list of an export).
fn entries_of(c: &AppContext, done: &Computed, now: Millis) -> Result<Vec<TimeEntry>> {
    let f = &c.settings.format;
    let filter = done.state.filter(done.today, f.week_start, f.tz);
    ReportService::filtered_entries(&c.db, &filter, &c.settings, now)
}

fn labels(tr: &Tr<'_>) -> Labels {
    Labels {
        months: [
            tr.get_month_jan(),
            tr.get_month_feb(),
            tr.get_month_mar(),
            tr.get_month_apr(),
            tr.get_month_may(),
            tr.get_month_jun(),
            tr.get_month_jul(),
            tr.get_month_aug(),
            tr.get_month_sep(),
            tr.get_month_oct(),
            tr.get_month_nov(),
            tr.get_month_dec(),
        ]
        .map(|s| s.to_string()),
        total: tr.get_report_total().to_string(),
        other: tr.get_report_other().to_string(),
        no_project: tr.get_no_project().to_string(),
        no_task: tr.get_no_task().to_string(),
        no_client: tr.get_no_client().to_string(),
        no_tag: tr.get_no_tag().to_string(),
        no_description: tr.get_no_description().to_string(),
        billable: tr.get_group_billable().to_string(),
        non_billable: tr.get_non_billable().to_string(),
    }
}

fn export_labels(tr: &Tr<'_>) -> ExportLabels {
    ExportLabels {
        group: tr.get_export_group().into(),
        subgroup: tr.get_export_subgroup().into(),
        duration: tr.get_export_duration().into(),
        hours: tr.get_export_hours().into(),
        cost: tr.get_export_cost().into(),
        total: tr.get_export_total().into(),
        no_project: tr.get_export_no_project().into(),
        no_task: tr.get_export_no_task().into(),
        no_client: tr.get_export_no_client().into(),
        no_tag: tr.get_export_no_tag().into(),
        yes: tr.get_export_yes().into(),
        no: tr.get_export_no().into(),
        description: tr.get_export_description().into(),
        project: tr.get_export_project().into(),
        task: tr.get_export_task().into(),
        client: tr.get_export_client().into(),
        tags: tr.get_export_tags().into(),
        billable: tr.get_export_billable().into(),
        start: tr.get_export_start().into(),
        end: tr.get_export_end().into(),
        date: tr.get_export_date().into(),
        summary_sheet: tr.get_export_sheet_summary().into(),
        entries_sheet: tr.get_export_sheet_entries().into(),
    }
}

/// Name of a grouping in the group selects.
fn group_label(tr: &Tr<'_>, g: GroupBy) -> SharedString {
    match g {
        GroupBy::Project => tr.get_group_project(),
        GroupBy::Task => tr.get_group_task(),
        GroupBy::Client => tr.get_group_client(),
        GroupBy::Tag => tr.get_group_tag(),
        GroupBy::Billable => tr.get_group_billable(),
        GroupBy::Description => tr.get_group_description(),
        GroupBy::Day => tr.get_group_day(),
    }
}

/// Options of a group select: all groupings, or (with `without`) "None" and every grouping
/// but `without`.
fn group_options(tr: &Tr<'_>, without: Option<GroupBy>) -> Vec<SelectOption> {
    let none = without.map(|_| SelectOption {
        value: -1,
        label: tr.get_group_none(),
    });
    none.into_iter()
        .chain(
            GROUPS
                .iter()
                .filter(|g| Some(**g) != without)
                .map(|g| SelectOption {
                    value: group_value(*g),
                    label: group_label(tr, *g),
                }),
        )
        .collect()
}

/// Everything an export file contains.
struct ExportData<'a> {
    result: &'a ReportResult,
    entries: &'a [TimeEntry],
    data: &'a Data,
    g1: GroupBy,
    g2: Option<GroupBy>,
    labels: Labels,
    export_labels: ExportLabels,
    prefs: FormatPrefs,
}

/// Writes the export to `path`.
fn write_export(kind: Export, path: &Path, d: &ExportData) -> Result<()> {
    let names = Names {
        catalog: &d.data.catalog,
        clients: &d.data.clients,
        labels: &d.labels,
        date: d.prefs.date,
    };
    // The exports ask for a name per key without saying which grouping it belongs to; the
    // keys of the first level are the very `GroupKey`s of `result.groups`.
    let name = |key: &GroupKey| {
        let first = d.result.groups.iter().any(|g| std::ptr::eq(&g.key, key));
        names.name(if first { d.g1 } else { d.g2.unwrap_or(d.g1) }, key)
    };
    let c = &d.data.catalog;
    let projects: HashMap<_, _> = c.projects.iter().map(|p| (p.id, p.clone())).collect();
    let tasks: HashMap<_, _> = c.tasks.iter().map(|t| (t.id, t.clone())).collect();
    let clients: HashMap<_, _> = d.data.clients.iter().map(|x| (x.id, x.clone())).collect();
    let tags: HashMap<_, _> = c.tags.iter().map(|t| (t.id, t.clone())).collect();
    let lookup = Lookup {
        projects: &projects,
        tasks: &tasks,
        clients: &clients,
        tags: &tags,
    };
    match kind {
        Export::ReportCsv => {
            let file = BufWriter::new(File::create(path).map_err(ServiceError::Io)?);
            export_report_csv(file, d.result, &name, &d.export_labels, &d.prefs)
        }
        Export::EntriesCsv => {
            let file = BufWriter::new(File::create(path).map_err(ServiceError::Io)?);
            export_entries_csv(file, d.entries, &lookup, &d.export_labels, &d.prefs)
        }
        Export::Xlsx => export_report_xlsx(
            path,
            d.result,
            &name,
            d.entries,
            &lookup,
            &d.export_labels,
            &d.prefs,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use tt_domain::{EntryTemplate, RoundingMode};
    use tt_services::NewProject;

    fn ctx(dir: &tempfile::TempDir) -> Ctx {
        Rc::new(RefCell::new(
            AppContext::open(dir.path().join("tt.db")).unwrap(),
        ))
    }

    fn controller(ctx: &Ctx) -> Rc<ReportsController> {
        let ctl = Rc::new(ReportsController {
            ctx: ctx.clone(),
            ui: slint::Weak::default(),
            state: RefCell::default(),
            saved: RefCell::default(),
            bars: Rc::default(),
            pie: Rc::default(),
            rows: Rc::default(),
            runs: Cell::new(0),
        });
        ctl.listen_to_entries();
        ctl
    }

    /// Two entries yesterday: 1 h 10 min of "Site" › "Login" and 20 min without project.
    /// Returns the project and task ids.
    fn seed(ctx: &Ctx) -> (Id, Id) {
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
                    is_billable: true,
                    billable_rate: Some(6000),
                    estimated_time: None,
                },
            )
            .unwrap();
        let task = c
            .catalog
            .create_task(&mut c.db, project.id, "Login", None)
            .unwrap();
        let tz = c.settings.format.tz;
        let day = local_date(c.timer.clock().now(), tz) - chrono::Days::new(1);
        let at = |h, m| crate::vm::entries::local_millis(day, h, m, tz);
        let with = EntryTemplate {
            description: "=cmd".into(),
            project_id: Some(project.id),
            task_id: Some(task.id),
            billable: true,
            ..EntryTemplate::default()
        };
        c.entries
            .create_manual(&mut c.db, with, at(9, 0), at(10, 10), tz, false)
            .unwrap();
        let without = EntryTemplate {
            description: "misc".into(),
            ..EntryTemplate::default()
        };
        c.entries
            .create_manual(&mut c.db, without, at(11, 0), at(11, 20), tz, false)
            .unwrap();
        (project.id, task.id)
    }

    fn yesterday() -> ReportState {
        let mut s = ReportState::default();
        s.set_preset_value(1);
        s
    }

    #[test]
    fn compute_runs_the_report_of_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let (project, task) = seed(&ctx);
        let c = ctx.borrow();
        let now = c.timer.clock().now();

        let done = compute(&c, yesterday(), now).unwrap();
        assert_eq!(done.result.total_seconds, 90 * 60);
        assert_eq!(done.result.groups.len(), 2);
        assert_eq!(done.result.groups[0].key, GroupKey::Id(project));
        assert_eq!(done.result.groups[0].children[0].key, GroupKey::Id(task));
        assert_eq!(done.range.0, done.range.1);

        // Filtering and rounding (up to 30 min: the 70 min of the project become 90).
        let mut s = yesterday();
        s.project_ids = vec![project];
        s.rounding = Some((RoundingMode::Up, 30));
        let done = compute(&c, s.clone(), now).unwrap();
        assert_eq!(done.result.total_seconds, 90 * 60);
        s.billable = Some(false);
        let done = compute(&c, s, now).unwrap();
        assert_eq!(done.result.total_seconds, 0);

        // A project deleted since the state was saved is dropped from the filter.
        let mut s = yesterday();
        s.project_ids = vec![tt_domain::new_id()];
        let done = compute(&c, s, now).unwrap();
        assert!(done.state.project_ids.is_empty());
        assert_eq!(done.result.total_seconds, 90 * 60);
    }

    #[test]
    fn rounding_changes_the_total() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        seed(&ctx);
        let c = ctx.borrow();
        let now = c.timer.clock().now();
        let plain = compute(&c, yesterday(), now).unwrap().result.total_seconds;
        let mut s = yesterday();
        s.rounding = Some((RoundingMode::Up, 60));
        let up = compute(&c, s, now).unwrap().result.total_seconds;
        assert_eq!(plain, 90 * 60);
        assert_eq!(up, 3 * 3600); // 70 min -> 2 h, 20 min -> 1 h
    }

    #[test]
    fn the_state_is_saved_in_the_settings_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let ctl = controller(&ctx);
        let mut s = yesterday();
        s.g1 = GroupBy::Day;
        s.g2 = None;
        ctl.save(&s);
        let stored = settings::get(ctx.borrow().db.conn(), SETTING_KEY).unwrap();
        assert_eq!(ReportState::parse(stored.as_deref()), s);
        assert_eq!(*ctl.saved.borrow(), s.to_json());

        // Garbage in the settings gives the defaults.
        settings::set(ctx.borrow().db.conn(), SETTING_KEY, "{broken").unwrap();
        let stored = settings::get(ctx.borrow().db.conn(), SETTING_KEY).unwrap();
        assert_eq!(
            ReportState::parse(stored.as_deref()),
            ReportState::default()
        );
    }

    #[test]
    fn entry_events_only_run_the_report_of_a_visible_page() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        let ctl = controller(&ctx);
        // No window = the reports are not the open page.
        let events = ctx.borrow().events.clone();
        events.emit_entries_changed();
        assert_eq!(ctl.runs.get(), 0);
    }

    fn labels_en() -> Labels {
        let s = |x: &str| x.to_owned();
        Labels {
            months: std::array::from_fn(|i| format!("M{}", i + 1)),
            total: s("Total"),
            other: s("Other"),
            no_project: s("No project"),
            no_task: s("No task"),
            no_client: s("No client"),
            no_tag: s("No tag"),
            no_description: s("No description"),
            billable: s("Billable"),
            non_billable: s("Non-billable"),
        }
    }

    fn export_labels_en() -> ExportLabels {
        let s = |x: &str| x.to_owned();
        ExportLabels {
            group: s("Group"),
            subgroup: s("Subgroup"),
            duration: s("Duration"),
            hours: s("Duration (h)"),
            cost: s("Cost"),
            total: s("Total"),
            no_project: s("No project"),
            no_task: s("No task"),
            no_client: s("No client"),
            no_tag: s("No tag"),
            yes: s("Yes"),
            no: s("No"),
            description: s("Description"),
            project: s("Project"),
            task: s("Task"),
            client: s("Client"),
            tags: s("Tags"),
            billable: s("Billable"),
            start: s("Start"),
            end: s("End"),
            date: s("Date"),
            summary_sheet: s("Summary"),
            entries_sheet: s("Entries"),
        }
    }

    #[test]
    fn exports_write_a_bom_csv_the_entries_and_a_two_sheet_xlsx() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(&dir);
        seed(&ctx);
        let c = ctx.borrow();
        let now = c.timer.clock().now();
        let done = compute(&c, yesterday(), now).unwrap();
        let entries = entries_of(&c, &done, now).unwrap();
        assert_eq!(entries.len(), 2);
        let data = ExportData {
            result: &done.result,
            entries: &entries,
            data: &done.data,
            g1: done.state.g1,
            g2: done.state.g2,
            labels: labels_en(),
            export_labels: export_labels_en(),
            prefs: FormatPrefs::default(),
        };

        let csv = dir.path().join("report.csv");
        write_export(Export::ReportCsv, &csv, &data).unwrap();
        let bytes = std::fs::read(&csv).unwrap();
        assert_eq!(&bytes[..3], [0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Group,Subgroup,Duration,Cost");
        // Group names per level: the project, its task, "No project" with "No task".
        assert!(lines.contains(&"Site,,1:10,$70.00"), "{text}");
        assert!(lines.contains(&"Site,Site › Login,1:10,$70.00"), "{text}");
        assert!(lines.contains(&"No project,,0:20,$0.00"), "{text}");
        assert!(lines.contains(&"No project,No task,0:20,$0.00"), "{text}");
        assert_eq!(*lines.last().unwrap(), "Total,,1:30,$70.00");

        let csv = dir.path().join("entries.csv");
        write_export(Export::EntriesCsv, &csv, &data).unwrap();
        let text = String::from_utf8(std::fs::read(&csv).unwrap()[3..].to_vec()).unwrap();
        assert_eq!(text.lines().count(), 3, "{text}");
        // A formula-looking description is defused.
        assert!(text.contains("'=cmd"), "{text}");

        let xlsx = dir.path().join("report.xlsx");
        write_export(Export::Xlsx, &xlsx, &data).unwrap();
        let bytes = std::fs::read(&xlsx).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        let has = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(has("xl/worksheets/sheet1.xml") && has("xl/worksheets/sheet2.xml"));
        assert!(!has("xl/worksheets/sheet3.xml"));
    }
}
