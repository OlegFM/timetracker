//! Time screen: the entries list by day (paged by [`BATCH`] days), row actions, and the
//! entry dialog (edit / manual add).
//!
//! The list reloads the loaded range whenever entries change (`AppEvents`), shows the
//! running entry's duration live, and after its own changes re-pushes the timer bar
//! ([`TimerHub::refresh`]) and tells the other listeners.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use chrono::NaiveDate;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use tt_domain::{FormatPrefs, Id, Millis, TimeEntry, TimeFormat, Tz, format_duration, local_date};
use tt_services::{Clock, Result, ServiceError, default_billable};

use crate::app::{AppContext, Ctx};
use crate::controllers::timer::TimerHub;
use crate::controllers::{error_message, sync_vec};
use crate::vm::entries::{
    Day, day_title, day_total_ms, dialog_range_from, duration_between, edit_view, end_after,
    entry_row, new_entry_view, normalize_duration, normalize_time, page_bound, reload_days,
    template_of,
};
use crate::vm::error::error_code;
use crate::vm::format::prefs_from;
use crate::vm::pickers::parse_id;
use crate::{
    DayView, EntriesBridge, EntryDialogData, EntryDialogView, EntryRowView, MainWindow, ToastKind,
    Tr,
};

/// Days loaded at first and per `load-more`.
pub const BATCH: usize = 14;

/// How far back the list goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    /// Nothing loaded yet.
    Initial,
    /// Days from this date on are loaded; older ones may exist.
    Since(NaiveDate),
    /// Every day is loaded.
    All,
}

fn bound_after(page: &[Day]) -> Bound {
    page_bound(page, BATCH).map_or(Bound::All, Bound::Since)
}

#[derive(Default)]
struct Shown {
    days: Vec<Day>,
    /// Row models of `days` (same order).
    rows: Vec<Rc<VecModel<EntryRowView>>>,
    /// Day and row index of the running entry.
    running: Option<(usize, usize)>,
}

pub struct EntriesController {
    ctx: Ctx,
    hub: Rc<TimerHub>,
    ui: slint::Weak<MainWindow>,
    me: Weak<EntriesController>,
    bound: RefCell<Bound>,
    shown: RefCell<Shown>,
    days: Rc<VecModel<DayView>>,
    /// Runs while a running entry is shown.
    ticker: Timer,
}

/// Wires `EntriesBridge` of the main window and loads the first days.
pub fn install(ui: &MainWindow, ctx: &Ctx, hub: &Rc<TimerHub>) -> Rc<EntriesController> {
    let ctl = Rc::new_cyclic(|me| EntriesController {
        ctx: ctx.clone(),
        hub: hub.clone(),
        ui: ui.as_weak(),
        me: me.clone(),
        bound: RefCell::new(Bound::Initial),
        shown: RefCell::default(),
        days: Rc::new(VecModel::default()),
        ticker: Timer::default(),
    });
    let b = ui.global::<EntriesBridge>();
    b.set_days(ModelRc::from(ctl.days.clone()));

    let on = |f: fn(&EntriesController, &str)| {
        let c = ctl.clone();
        move |id: SharedString| f(&c, &id)
    };
    b.on_continue(on(EntriesController::continue_entry));
    b.on_edit(on(EntriesController::edit));
    b.on_duplicate(on(EntriesController::duplicate));
    b.on_delete(on(EntriesController::delete));
    let c = ctl.clone();
    b.on_load_more(move || c.load_more());
    let c = ctl.clone();
    b.on_add_manual(move || c.add_manual());
    let c = ctl.clone();
    b.on_save(move |data| c.save(&data));
    let c = ctl.clone();
    b.on_project_billable(move |id| c.project_billable(&id));

    let c = ctl.clone();
    b.on_normalize_time(move |s| normalize_time(&s, c.time_format()).into());
    b.on_normalize_duration(|s| normalize_duration(&s).into());
    b.on_duration_between(|s, e| duration_between(&s, &e).into());
    let c = ctl.clone();
    b.on_end_after(move |s, d| end_after(&s, &d, c.time_format()).into());

    let me = Rc::downgrade(&ctl);
    let events = ctx.borrow().events.clone();
    events.on_data_changed(move || {
        if let Some(c) = me.upgrade() {
            c.reload();
        }
    });
    ctl.reload();
    ctl
}

impl EntriesController {
    fn reload(&self) {
        let bound = *self.bound.borrow();
        let res = self.with_ctx(|c, tz, now| {
            let fetch = |before| c.entries.list_days(&c.db, before, BATCH as u32, tz, now);
            match bound {
                Bound::Initial => {
                    let page = fetch(None)?;
                    let b = bound_after(&page);
                    Ok((page, b))
                }
                Bound::Since(d) => reload_days(Some(d), BATCH, fetch)
                    .map(|(days, s)| (days, s.map_or(Bound::All, Bound::Since))),
                Bound::All => reload_days(None, BATCH, fetch).map(|(days, _)| (days, Bound::All)),
            }
        });
        match res {
            Ok((days, b)) => {
                self.bound.replace(b);
                self.show(days);
            }
            Err(e) => self.fail("load", &e),
        }
    }

    fn load_more(&self) {
        let Bound::Since(oldest) = *self.bound.borrow() else {
            return;
        };
        let res = self.with_ctx(|c, tz, now| {
            c.entries
                .list_days(&c.db, Some(oldest), BATCH as u32, tz, now)
        });
        match res {
            Ok(page) => {
                self.bound.replace(bound_after(&page));
                let mut days = self.shown.borrow().days.clone();
                days.extend(page);
                self.show(days);
            }
            Err(e) => self.fail("load more", &e),
        }
    }

    /// Shows `days`, updating the models in place (rows that did not change are not
    /// touched, so the list keeps its scroll position and appends on load-more).
    fn show(&self, days: Vec<Day>) {
        let (prefs, labels) = self.prefs_and_labels();
        let now = self.now();
        let today = local_date(now, prefs.tz);
        let mut row_models = std::mem::take(&mut self.shown.borrow_mut().rows);
        row_models.truncate(days.len());
        let mut running = None;
        let mut views = Vec::with_capacity(days.len());
        self.hub.with_catalog(|catalog| {
            for (di, (date, list, _)) in days.iter().enumerate() {
                let rows: Vec<EntryRowView> = list
                    .iter()
                    .map(|e| entry_row(e, catalog, &prefs, now))
                    .collect();
                if let Some(ri) = list.iter().position(|e| e.end.is_none()) {
                    running = Some((di, ri));
                }
                if di == row_models.len() {
                    row_models.push(Rc::new(VecModel::default()));
                }
                sync_vec(&row_models[di], rows);
                views.push(DayView {
                    title: day_title(*date, today, prefs.date, &labels.0, &labels.1).into(),
                    total: total_text(list, now, &prefs),
                    rows: ModelRc::from(row_models[di].clone()),
                });
            }
        });
        self.shown.replace(Shown {
            days,
            rows: row_models,
            running,
        });
        sync_vec(&self.days, views);
        let has_more = matches!(*self.bound.borrow(), Bound::Since(_) | Bound::Initial);
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<EntriesBridge>().set_has_more(has_more);
        }
        self.set_ticking(running.is_some());
    }

    /// Updates the running row's duration and its day total.
    fn tick(&self) {
        let shown = self.shown.borrow();
        let Some((di, ri)) = shown.running else {
            return;
        };
        let (prefs, _) = self.prefs_and_labels();
        let now = self.now();
        let (list, rows) = (&shown.days[di].1, &shown.rows[di]);
        if let Some(mut row) = rows.row_data(ri) {
            let duration: SharedString =
                format_duration(list[ri].duration_ms(now) / 1000, &prefs).into();
            if row.duration != duration {
                row.duration = duration;
                rows.set_row_data(ri, row);
            }
        }
        if let Some(mut day) = self.days.row_data(di) {
            let total = total_text(list, now, &prefs);
            if day.total != total {
                day.total = total;
                self.days.set_row_data(di, day);
            }
        }
    }

    fn set_ticking(&self, on: bool) {
        if on && !self.ticker.running() {
            let me = self.me.clone();
            self.ticker
                .start(TimerMode::Repeated, Duration::from_secs(1), move || {
                    if let Some(c) = me.upgrade() {
                        c.tick();
                    }
                });
        } else if !on && self.ticker.running() {
            self.ticker.stop();
        }
    }

    fn find(&self, id: &str) -> Option<TimeEntry> {
        let id = parse_id(id)?;
        self.shown
            .borrow()
            .days
            .iter()
            .flat_map(|d| d.1.iter())
            .find(|e| e.id == id)
            .cloned()
    }

    fn continue_entry(&self, id: &str) {
        let res = self.with_id(id, |c, tz, _, id| {
            c.entries
                .continue_entry(&mut c.db, &c.timer, id, tz)
                .map(drop)
        });
        self.after("continue", res);
    }

    fn duplicate(&self, id: &str) {
        let res = self.with_id(id, |c, _, prevent, id| {
            c.entries.duplicate(&mut c.db, id, prevent).map(drop)
        });
        self.after("duplicate", res);
    }

    fn delete(&self, id: &str) {
        let res = self.with_id(id, |c, _, _, id| c.entries.delete(&mut c.db, id));
        self.after("delete", res);
    }

    fn edit(&self, id: &str) {
        let Some(e) = self.find(id) else {
            // Stale row: show the stored state.
            self.reload();
            return;
        };
        let (prefs, _) = self.prefs_and_labels();
        let now = self.now();
        let view = self.hub.with_catalog(|c| edit_view(&e, c, &prefs, now));
        self.open_dialog(view);
    }

    fn add_manual(&self) {
        let (prefs, _) = self.prefs_and_labels();
        self.open_dialog(new_entry_view(self.now(), &prefs));
    }

    fn open_dialog(&self, view: EntryDialogView) {
        if let Some(ui) = self.ui.upgrade() {
            let b = ui.global::<EntriesBridge>();
            b.set_dialog(view);
            b.set_dialog_open(true);
        }
    }

    /// Saves the dialog; "" on success, else the message for the dialog.
    fn save(&self, d: &EntryDialogData) -> SharedString {
        let (tz, now) = (self.ctx.borrow().settings.format.tz, self.now());
        // Unchanged fields keep the entry's exact instants (seconds included).
        let original = self.find(&d.id).map(|e| (e.start, e.end));
        let Some((start, end)) = dialog_range_from(&d.date, &d.start, &d.end, tz, original) else {
            return self.tr(|tr| tr.get_err_generic());
        };
        let t = template_of(d);
        let entry = |id: Id, end: Option<Millis>| TimeEntry {
            id,
            session_id: None,
            description: t.description.clone(),
            start,
            end,
            billable: t.billable,
            project_id: t.project_id,
            task_id: t.task_id,
            tag_ids: t.tag_ids.clone(),
            created_at: 0,
            updated_at: 0,
        };
        let res = match (parse_id(&d.id), end) {
            (None, Some(end)) => self.with_ctx(|c, tz, _| {
                let prevent = c.settings.prevent_overlap;
                c.entries
                    .create_manual(&mut c.db, t.clone(), start, end, tz, prevent)
                    .map(drop)
            }),
            (Some(id), Some(_)) => self.with_ctx(|c, tz, _| {
                let prevent = c.settings.prevent_overlap;
                c.entries
                    .update(&mut c.db, entry(id, end), tz, prevent)
                    .map(drop)
            }),
            (Some(_), None) if start > now => {
                return self.tr(|tr| tr.get_err_start_future());
            }
            // Ruling R22: start (and attributes) via the entry, the session template via
            // the timer, then the midnight split (R15).
            (Some(id), None) => self.with_ctx(|c, tz, _| {
                let prevent = c.settings.prevent_overlap;
                c.entries.update(&mut c.db, entry(id, None), tz, prevent)?;
                if c.timer.status(&c.db, tz)?.session.is_some() {
                    c.timer.update_template(&mut c.db, t.clone(), tz)?;
                }
                c.timer.rollover(&mut c.db, tz).map(drop)
            }),
            (None, None) => Err(ServiceError::Invalid("range")),
        };
        match res {
            Ok(()) => {
                self.changed();
                SharedString::new()
            }
            Err(e) => {
                log::warn!("entries save failed: {}", error_code(&e));
                self.tr(|tr| error_message(tr, &e))
            }
        }
    }

    fn project_billable(&self, id: &str) -> bool {
        let Some(id) = parse_id(id) else {
            return false;
        };
        self.hub.with_catalog(|c| default_billable(c.project(id)))
    }

    /// After a successful change: the timer bar and every list show the stored state.
    fn changed(&self) {
        self.hub.refresh();
        let events = self.ctx.borrow().events.clone();
        events.emit_entries_changed();
    }

    fn after(&self, op: &'static str, res: Result<()>) {
        match res {
            Ok(()) => self.changed(),
            Err(e) => self.fail(op, &e),
        }
    }

    /// Logs (code only), shows the error and re-syncs the list with the stored state.
    fn fail(&self, op: &str, e: &ServiceError) {
        log::warn!("entries {op} failed: {}", error_code(e));
        if let Some(ui) = self.ui.upgrade() {
            let text = error_message(&ui.global::<Tr>(), e);
            ui.invoke_show_toast(text, ToastKind::Error);
        }
        if op != "load" {
            self.reload();
        }
    }

    /// Runs `f` for the entry id `id` with the overlap setting.
    fn with_id(
        &self,
        id: &str,
        f: impl FnOnce(&mut AppContext, Tz, bool, Id) -> Result<()>,
    ) -> Result<()> {
        let id = parse_id(id).ok_or(ServiceError::NotFound)?;
        self.with_ctx(|c, tz, _| {
            let prevent = c.settings.prevent_overlap;
            f(c, tz, prevent, id)
        })
    }

    /// Runs `f` with the context, time zone and current time (borrow ends on return).
    fn with_ctx<T>(&self, f: impl FnOnce(&mut AppContext, Tz, Millis) -> Result<T>) -> Result<T> {
        let mut c = self.ctx.borrow_mut();
        let (tz, now) = (c.settings.format.tz, c.timer.clock().now());
        f(&mut c, tz, now)
    }

    fn now(&self) -> Millis {
        self.ctx.borrow().timer.clock().now()
    }

    fn time_format(&self) -> TimeFormat {
        self.ctx.borrow().settings.format.time
    }

    fn tr(&self, f: impl FnOnce(&Tr<'_>) -> SharedString) -> SharedString {
        self.ui
            .upgrade()
            .map(|ui| f(&ui.global::<Tr>()))
            .unwrap_or_default()
    }

    /// Format preferences with localized suffixes, and the ("Today", "Yesterday") titles.
    fn prefs_and_labels(&self) -> (FormatPrefs, (SharedString, SharedString)) {
        let (hour, minute, today, yesterday) = self
            .ui
            .upgrade()
            .map(|ui| {
                let tr = ui.global::<Tr>();
                (
                    tr.get_hour_suffix(),
                    tr.get_minute_suffix(),
                    tr.get_day_today(),
                    tr.get_day_yesterday(),
                )
            })
            .unwrap_or_default();
        let prefs = prefs_from(&self.ctx.borrow().settings, &hour, &minute);
        (prefs, (today, yesterday))
    }
}

fn total_text(list: &[TimeEntry], now: Millis, prefs: &FormatPrefs) -> SharedString {
    format_duration(day_total_ms(list, now) / 1000, prefs).into()
}
