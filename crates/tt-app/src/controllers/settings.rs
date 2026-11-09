//! Settings screen and its side effects. Every change is applied at once, saved with
//! `SettingsService::save` and announced with `AppEvents::emit_settings_changed` (outside
//! any context borrow) so that every controller draws its data again.
//!
//! Also here: backup / restore of the database file and the (build-time optional) update
//! check. Network calls run on a worker thread and report back through the event loop.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use slint::{ComponentHandle, SharedString};
use tt_domain::{Tz, local_date};
use tt_services::update::{Manifest, check_for_update, download_and_verify, update_config};
use tt_services::{
    Clock, ServiceError, Settings, SettingsService, Theme,
    backup::{backup_to, restore_from},
};
use tt_storage::Db;

use crate::app::{self, Ctx};
use crate::controllers::error_message;
use crate::controllers::mini::MiniController;
use crate::controllers::pickers::model;
use crate::controllers::timer::TimerHub;
use crate::platform::{autostart, fatal};
use crate::vm::catalog::{money_text, parse_rate};
use crate::vm::error::error_code;
use crate::vm::format::prefs_from;
use crate::vm::lang;
use crate::vm::settings::{
    CURRENCIES, DATES, INTERVALS, NUMBERS, TIMES, WEEK_STARTS, choice, clamp_transparency,
    examples, index_of, select_options, timezone_options, valid_currency_code,
};
use crate::{MainWindow, SettingsBridge, ToastKind, Tr};

/// `SettingsBridge.update-state` values.
const UPDATE_IDLE: i32 = 0;
const UPDATE_CHECKING: i32 = 1;
const UPDATE_LATEST: i32 = 2;
const UPDATE_AVAILABLE: i32 = 3;
const UPDATE_FAILED: i32 = 4;
const UPDATE_DOWNLOADING: i32 = 5;

/// Name of the language in that language (never translated, so a user who ended up in a
/// language they cannot read can still find their own). `None` for an unsupported code.
pub fn language_label(code: &str) -> Option<&'static str> {
    match code {
        "en" => Some("English"),
        "ru" => Some("Русский"),
        "zh_CN" => Some("中文 (简体)"),
        "es" => Some("Español"),
        _ => None,
    }
}

/// Idle opacity of the mini timer for a transparency in percent (70 % is 0.3).
pub fn opacity_from_transparency(percent: i32) -> f32 {
    (100 - percent.clamp(0, 100)) as f32 / 100.0
}

/// Transparency in percent as the slider shows it (0 to 90, steps of 5) for an opacity.
pub fn transparency_from_opacity(opacity: f32) -> i32 {
    clamp_transparency(((1.0 - opacity) * 100.0).round() as i32)
}

/// Default name of a backup file.
pub fn backup_file_name(today: NaiveDate) -> String {
    format!("timetracker-backup-{}.db", today.format("%Y-%m-%d"))
}

pub struct SettingsController {
    ctx: Ctx,
    hub: Rc<TimerHub>,
    mini: Rc<MiniController>,
    ui: slint::Weak<MainWindow>,
    /// The backup file chosen for a restore, until it is confirmed or cancelled.
    pending_restore: RefCell<Option<PathBuf>>,
    /// The newer release found by the last update check.
    update: Arc<Mutex<Option<Manifest>>>,
}

/// Wires `SettingsBridge` of the main window. Call after the mini timer exists.
pub fn install(
    ui: &MainWindow,
    ctx: &Ctx,
    hub: &Rc<TimerHub>,
    mini: &Rc<MiniController>,
) -> Rc<SettingsController> {
    let ctl = Rc::new(SettingsController {
        ctx: ctx.clone(),
        hub: hub.clone(),
        mini: mini.clone(),
        ui: ui.as_weak(),
        pending_restore: RefCell::default(),
        update: Arc::default(),
    });
    let b = ui.global::<SettingsBridge>();
    b.set_languages(model(select_options(
        lang::SUPPORTED
            .iter()
            .map(|c| language_label(c).unwrap_or(c).to_owned())
            .collect(),
    )));
    b.set_timezones(model(timezone_options()));
    b.set_app_version(env!("CARGO_PKG_VERSION").into());
    b.set_updates_enabled(update_config().is_some());

    let on_index = |f: fn(&SettingsController, i32)| {
        let c = ctl.clone();
        move |i: i32| f(&c, i)
    };
    b.on_set_language(on_index(SettingsController::set_language));
    b.on_set_date(on_index(|c, i| c.set_date(i)));
    b.on_set_time(on_index(|c, i| c.set_time(i)));
    b.on_set_interval(on_index(|c, i| c.set_interval(i)));
    b.on_set_number(on_index(|c, i| c.set_number(i)));
    b.on_set_currency(on_index(|c, i| c.set_currency(i)));
    b.on_set_week_start(on_index(|c, i| c.set_week_start(i)));
    b.on_set_theme(on_index(SettingsController::set_theme));
    b.on_set_mini_transparency(on_index(SettingsController::set_mini_transparency));
    let on_flag = |f: fn(&SettingsController, bool)| {
        let c = ctl.clone();
        move |on: bool| f(&c, on)
    };
    b.on_set_close_to_tray(on_flag(|c, on| {
        c.change(|s| s.close_to_tray = on);
        c.push_values();
    }));
    b.on_set_prevent_overlap(on_flag(|c, on| {
        c.change(|s| s.prevent_overlap = on);
        c.push_values();
    }));
    b.on_set_autostart(on_flag(SettingsController::set_autostart));
    b.on_set_update_auto_check(on_flag(|c, on| {
        c.change(|s| s.update_auto_check = on);
        c.push_values();
    }));

    let c = ctl.clone();
    b.on_set_timezone(move |name| c.set_timezone(&name));
    let c = ctl.clone();
    b.on_set_currency_code(move |text| c.set_currency_code(&text));
    let c = ctl.clone();
    b.on_set_default_rate(move |text| c.set_default_rate(&text));
    let c = ctl.clone();
    b.on_sync_fields(move || c.sync_fields());
    let c = ctl.clone();
    b.on_backup(move || c.backup());
    let c = ctl.clone();
    b.on_restore(move || c.choose_backup());
    let c = ctl.clone();
    b.on_confirm_restore(move || c.confirm_restore());
    let c = ctl.clone();
    b.on_cancel_restore(move || c.cancel_restore());
    let c = ctl.clone();
    b.on_check_updates(move || c.check_updates(false));
    let c = ctl.clone();
    b.on_install_update(move || c.install_update());

    ctl.push_values();
    ctl.sync_fields();
    ctl.push_examples();
    if update_config().is_some() && ctx.borrow().settings.update_auto_check {
        ctl.check_updates(true);
    }
    ctl
}

impl SettingsController {
    // ---- applying changes -------------------------------------------------------------

    /// Changes the settings and saves them (an error becomes a toast; the change stays in
    /// effect for this run, like the other settings that are saved on the fly).
    fn update(&self, f: impl FnOnce(&mut Settings)) {
        let res = {
            let mut c = self.ctx.borrow_mut();
            f(&mut c.settings);
            c.save_settings()
        };
        if let Err(e) = res {
            log::warn!("settings not saved: {e}");
            self.toast_with(|tr| tr.get_err_generic(), ToastKind::Error);
        }
    }

    /// Shows the new examples and tells the other controllers that settings changed.
    fn announce(&self) {
        self.push_examples();
        let events = self.ctx.borrow().events.clone();
        events.emit_settings_changed();
    }

    fn change(&self, f: impl FnOnce(&mut Settings)) {
        self.update(f);
        self.announce();
    }

    fn set_language(&self, index: i32) {
        let Some(code) = usize::try_from(index)
            .ok()
            .and_then(|i| lang::SUPPORTED.get(i))
        else {
            return;
        };
        self.update(|s| s.language = (*code).to_owned());
        // The catalog first: the listeners read the translated `Tr` strings.
        app::apply_language(code);
        self.announce();
        self.push_values();
    }

    fn set_date(&self, index: i32) {
        if let Some(f) = choice(&DATES, index) {
            self.change(|s| s.format.date = f);
        }
        self.push_values();
    }

    fn set_time(&self, index: i32) {
        if let Some(f) = choice(&TIMES, index) {
            self.change(|s| s.format.time = f);
        }
        self.push_values();
    }

    fn set_interval(&self, index: i32) {
        if let Some(f) = choice(&INTERVALS, index) {
            self.change(|s| s.format.interval = f);
        }
        self.push_values();
    }

    fn set_number(&self, index: i32) {
        if let Some(f) = choice(&NUMBERS, index) {
            self.change(|s| s.format.number = f);
        }
        self.push_values();
        // The rate field shows its amount with the number format.
        self.sync_fields();
    }

    fn set_currency(&self, index: i32) {
        if let Some(f) = choice(&CURRENCIES, index) {
            self.change(|s| s.format.currency = f);
        }
        self.push_values();
    }

    fn set_week_start(&self, index: i32) {
        if let Some(d) = choice(&WEEK_STARTS, index) {
            self.change(|s| s.format.week_start = d);
        }
        self.push_values();
    }

    fn set_timezone(&self, name: &str) {
        if let Ok(tz) = name.parse::<Tz>() {
            self.change(|s| s.format.tz = tz);
        }
        self.push_values();
    }

    fn set_theme(&self, index: i32) {
        let theme = if index == 1 {
            Theme::Light
        } else {
            Theme::Dark
        };
        self.update(|s| s.theme = theme);
        if let Some(ui) = self.ui.upgrade() {
            app::apply_theme(&ui, theme);
        }
        self.mini.apply_appearance();
        self.announce();
        self.push_values();
    }

    fn set_mini_transparency(&self, percent: i32) {
        let percent = clamp_transparency(percent);
        self.update(|s| s.mini_timer_idle_opacity = opacity_from_transparency(percent));
        self.mini.apply_appearance();
        // Nothing but the mini timer shows it: no `settings changed` for every slider step.
        self.push_values();
    }

    fn set_autostart(&self, on: bool) {
        let registry = if self.ctx.borrow().manages_autostart {
            autostart::set_autostart(on)
        } else {
            Ok(()) // `--db`: the setting only
        };
        match registry {
            Ok(()) => self.change(|s| s.autostart = on),
            Err(e) => {
                log::warn!("autostart not changed: {e:#}");
                self.toast_with(|tr| tr.get_err_generic(), ToastKind::Error);
            }
        }
        self.push_values();
    }

    /// A code that is not three letters is reported and not saved.
    fn set_currency_code(&self, text: &str) {
        let code = valid_currency_code(text);
        if let Some(code) = &code
            && *code != self.ctx.borrow().settings.format.currency_code
        {
            self.change(|s| s.format.currency_code = code.clone());
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<SettingsBridge>()
                .set_currency_code_invalid(code.is_none());
        }
    }

    /// An empty field removes the rate; an unreadable amount is reported and not saved.
    fn set_default_rate(&self, text: &str) {
        let number = self.ctx.borrow().settings.format.number;
        let rate = parse_rate(text, number).ok();
        if let Some(rate) = rate
            && rate != self.ctx.borrow().settings.default_rate
        {
            self.change(|s| s.default_rate = rate);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<SettingsBridge>()
                .set_default_rate_invalid(rate.is_none());
        }
    }

    // ---- showing the state ------------------------------------------------------------

    /// Selects, toggles and the slider from the settings.
    fn push_values(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let c = self.ctx.borrow();
        let (s, f) = (&c.settings, &c.settings.format);
        let b = ui.global::<SettingsBridge>();
        b.set_language(index_of(&lang::SUPPORTED, &s.language.as_str()));
        b.set_date(index_of(&DATES, &f.date));
        b.set_time(index_of(&TIMES, &f.time));
        b.set_interval(index_of(&INTERVALS, &f.interval));
        b.set_number(index_of(&NUMBERS, &f.number));
        b.set_currency(index_of(&CURRENCIES, &f.currency));
        b.set_week_start(index_of(&WEEK_STARTS, &f.week_start));
        b.set_timezone(f.tz.name().into());
        b.set_theme(i32::from(s.theme == Theme::Light));
        b.set_mini_transparency(transparency_from_opacity(s.mini_timer_idle_opacity));
        b.set_close_to_tray(s.close_to_tray);
        b.set_autostart(s.autostart);
        b.set_prevent_overlap(s.prevent_overlap);
        b.set_update_auto_check(s.update_auto_check);
    }

    /// The two text fields show the saved values again and their errors go away.
    fn sync_fields(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let c = self.ctx.borrow();
        let b = ui.global::<SettingsBridge>();
        b.set_currency_code(c.settings.format.currency_code.clone().into());
        b.set_currency_code_invalid(false);
        let rate = c
            .settings
            .default_rate
            .map(|cents| money_text(cents, c.settings.format.number));
        b.set_default_rate(rate.unwrap_or_default().into());
        b.set_default_rate_invalid(false);
    }

    /// The format selects list each choice as an example of itself.
    fn push_examples(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let tr = ui.global::<Tr>();
        let prefs = prefs_from(
            &self.ctx.borrow().settings,
            &tr.get_hour_suffix(),
            &tr.get_minute_suffix(),
        );
        let e = examples(&prefs);
        let b = ui.global::<SettingsBridge>();
        b.set_date_options(model(select_options(e.dates)));
        b.set_time_options(model(select_options(e.times)));
        b.set_interval_options(model(select_options(e.intervals)));
        b.set_number_options(model(select_options(e.numbers)));
        b.set_currency_options(model(select_options(e.currencies)));
    }

    fn toast_with(&self, text: impl FnOnce(&Tr<'_>) -> SharedString, kind: ToastKind) {
        if let Some(ui) = self.ui.upgrade() {
            let text = text(&ui.global::<Tr>());
            ui.invoke_show_toast(text, kind);
        }
    }

    fn fail(&self, op: &str, e: &ServiceError) {
        log::warn!("settings {op} failed: {}", error_code(e));
        self.toast_with(|tr| error_message(tr, e), ToastKind::Error);
    }

    // ---- backup and restore -----------------------------------------------------------

    /// Asks where to save, then writes a consistent copy of the database there.
    fn backup(&self) {
        let today = {
            let c = self.ctx.borrow();
            local_date(c.timer.clock().now(), c.settings.format.tz)
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("TimeTracker", &["db"])
            .set_file_name(backup_file_name(today))
            .save_file()
        else {
            return;
        };
        let res = {
            let c = self.ctx.borrow();
            backup_to(&c.db, &path, c.timer.clock().now(), c.settings.format.tz)
        };
        match res {
            Ok(()) => self.toast_with(|tr| tr.get_backup_saved(), ToastKind::Success),
            Err(e) => self.fail("backup", &e),
        }
    }

    /// Asks for the backup file; the restore waits for the confirmation dialog.
    fn choose_backup(&self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("TimeTracker", &["db"])
            .pick_file()
        else {
            return;
        };
        self.pending_restore.replace(Some(path));
        self.set_restore_pending(true);
    }

    fn cancel_restore(&self) {
        self.pending_restore.take();
        self.set_restore_pending(false);
    }

    fn confirm_restore(&self) {
        self.set_restore_pending(false);
        let Some(path) = self.pending_restore.take() else {
            return;
        };
        match self.replace_database(&path) {
            Ok(()) => self.toast_with(|tr| tr.get_restored(), ToastKind::Success),
            Err(e) => self.fail("restore", &e),
        }
    }

    fn set_restore_pending(&self, on: bool) {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<SettingsBridge>().set_restore_pending(on);
        }
    }

    /// Replaces the live database with the backup `src` and shows the new data everywhere.
    ///
    /// The live file is released first (Windows cannot replace an open file), then
    /// `restore_from` validates and copies; the live path is opened again in every case,
    /// so a rejected backup leaves the data as it was.
    fn replace_database(&self, src: &Path) -> Result<(), ServiceError> {
        let live = self.ctx.borrow().db_path.clone();
        let placeholder = Db::open_in_memory()?;
        let live_db = std::mem::replace(&mut self.ctx.borrow_mut().db, placeholder);
        drop(live_db);
        let restored = restore_from(src, &live);
        match Db::open(&live) {
            Ok(db) => self.ctx.borrow_mut().db = db,
            Err(e) => {
                // Writing on into the placeholder would lose data silently.
                log::error!("database not reopened: {e}");
                let text = self
                    .ui
                    .upgrade()
                    .map(|ui| ui.global::<Tr>().get_db_reopen_failed())
                    .unwrap_or_default();
                fatal::show_error(&text);
                let _ = slint::quit_event_loop();
                return Err(e.into());
            }
        }
        restored?;

        let reloaded = {
            let mut c = self.ctx.borrow_mut();
            match SettingsService::load(&c.db) {
                Ok(s) => c.settings = s,
                Err(e) => log::warn!("restored settings not loaded: {}", error_code(&e)),
            }
            c.resolve_language(sys_locale::get_locale().as_deref())
                .map_err(|e| log::warn!("language not saved: {e}"))
                .ok()
        };
        self.show_restored_settings(reloaded);
        self.hub.reset();
        self.push_values();
        self.sync_fields();
        let events = self.ctx.borrow().events.clone();
        events.emit_settings_changed();
        events.emit_entries_changed();
        Ok(())
    }

    /// Applies the restored settings to the windows, the autostart entry and the mini timer.
    fn show_restored_settings(&self, language: Option<&'static str>) {
        let (theme, mini_visible, autostart_wanted) = {
            let s = &self.ctx.borrow().settings;
            (s.theme, s.mini_timer_visible, s.autostart)
        };
        if let Some(language) = language {
            app::apply_language(language);
        }
        if let Some(ui) = self.ui.upgrade() {
            app::apply_theme(&ui, theme);
        }
        self.mini.apply_appearance();
        if self.mini.is_visible() != mini_visible {
            self.mini.set_visible(mini_visible);
        }
        if !self.ctx.borrow().manages_autostart {
            return; // `--db`: the registry is left alone
        }
        match autostart::sync(autostart_wanted) {
            Ok(effective) if effective != autostart_wanted => {
                self.update(|s| s.autostart = effective);
            }
            Ok(_) => {}
            Err(e) => log::warn!("autostart not updated: {e:#}"),
        }
    }

    // ---- updates ----------------------------------------------------------------------

    fn set_update_state(&self, state: i32, version: &str) {
        if let Some(ui) = self.ui.upgrade() {
            let b = ui.global::<SettingsBridge>();
            b.set_update_state(state);
            b.set_update_version(version.into());
        }
    }

    /// Looks for a newer release on a worker thread. `quiet` (the check at start): only a
    /// found update is reported.
    fn check_updates(&self, quiet: bool) {
        let Some(cfg) = update_config() else { return };
        self.set_update_state(UPDATE_CHECKING, "");
        let (ui, slot) = (self.ui.clone(), self.update.clone());
        std::thread::spawn(move || {
            let found = check_for_update(&cfg, env!("CARGO_PKG_VERSION")).map_err(|e| {
                log::warn!("update check failed: {}", error_code(&e));
            });
            if let Ok(m) = &found {
                *slot.lock().unwrap_or_else(|p| p.into_inner()) = m.clone();
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui.upgrade() else { return };
                let (state, version) = match &found {
                    Ok(Some(m)) => (UPDATE_AVAILABLE, m.version.as_str()),
                    Ok(None) => (UPDATE_LATEST, ""),
                    Err(()) => (UPDATE_FAILED, ""),
                };
                let b = ui.global::<SettingsBridge>();
                b.set_update_state(if quiet && state != UPDATE_AVAILABLE {
                    UPDATE_IDLE
                } else {
                    state
                });
                b.set_update_version(version.into());
                let tr = ui.global::<Tr>();
                let text = match state {
                    UPDATE_AVAILABLE => tr.get_update_available(),
                    UPDATE_LATEST if !quiet => tr.get_update_latest(),
                    UPDATE_FAILED if !quiet => tr.get_update_failed(),
                    _ => return,
                };
                let kind = if state == UPDATE_FAILED {
                    ToastKind::Error
                } else {
                    ToastKind::Info
                };
                ui.invoke_show_toast(text, kind);
            });
        });
    }

    /// Downloads the found installer to the temp folder (checksum and signature are verified
    /// before the file appears), starts it and quits so that it can replace the program.
    fn install_update(&self) {
        let Some(cfg) = update_config() else { return };
        let Some(manifest) = self
            .update
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        else {
            return;
        };
        self.set_update_state(UPDATE_DOWNLOADING, &manifest.version);
        let ui = self.ui.clone();
        // A fixed name: the manifest is remote data and must not pick a path.
        let dest = std::env::temp_dir().join("TimeTracker-update.exe");
        std::thread::spawn(move || {
            let res = download_and_verify(&cfg, &manifest, &dest).map_err(|e| {
                log::warn!("update download failed: {}", error_code(&e));
            });
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui.upgrade() else { return };
                let started = res
                    .and_then(|()| std::process::Command::new(&dest).spawn().map_err(|_| ()))
                    .is_ok();
                let b = ui.global::<SettingsBridge>();
                if started {
                    let _ = slint::quit_event_loop();
                } else {
                    b.set_update_state(UPDATE_AVAILABLE);
                    let text = ui.global::<Tr>().get_err_generic();
                    ui.invoke_show_toast(text, ToastKind::Error);
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_is_one_minus_transparency() {
        assert_eq!(opacity_from_transparency(70), 0.3);
        assert_eq!(opacity_from_transparency(0), 1.0);
        assert_eq!(opacity_from_transparency(90), 0.1);
        // Out-of-range input is clamped, never negative or above 1.
        assert_eq!(opacity_from_transparency(-10), 1.0);
        assert_eq!(opacity_from_transparency(150), 0.0);
    }

    #[test]
    fn transparency_comes_back_from_the_stored_opacity() {
        assert_eq!(transparency_from_opacity(0.3), 70);
        assert_eq!(transparency_from_opacity(1.0), 0);
        assert_eq!(transparency_from_opacity(0.1), 90);
        // The default setting is the 70 % of the spec.
        assert_eq!(
            transparency_from_opacity(Settings::default().mini_timer_idle_opacity),
            70
        );
        // Values the slider cannot make (an older or edited file) snap to a step and to 90.
        assert_eq!(transparency_from_opacity(0.0), 90);
        assert_eq!(transparency_from_opacity(0.52), 50);
        for percent in (0..=90).step_by(5) {
            assert_eq!(
                transparency_from_opacity(opacity_from_transparency(percent)),
                percent
            );
        }
    }

    #[test]
    fn languages_are_named_in_themselves() {
        assert_eq!(language_label("en"), Some("English"));
        assert_eq!(language_label("ru"), Some("Русский"));
        assert_eq!(language_label("zh_CN"), Some("中文 (简体)"));
        assert_eq!(language_label("es"), Some("Español"));
        assert_eq!(language_label("fr"), None);
        // Every supported language has a label, and the label is not a translatable string.
        assert!(lang::SUPPORTED.iter().all(|c| language_label(c).is_some()));
        let ui = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/strings.slint"))
            .unwrap();
        assert!(!ui.contains("Русский") && !ui.contains("Español"));
    }

    #[test]
    fn backup_name_has_the_date() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(backup_file_name(day), "timetracker-backup-2026-11-09.db");
    }
}
