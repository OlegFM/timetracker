//! Checking for and installing a newer release (build-time optional, see
//! `tt_services::update`). Network calls run on a worker thread and report back through the
//! event loop. The state is shown through `SettingsBridge` (`update-state`, `update-version`).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::ComponentHandle;
use tt_services::update::{
    Manifest, check_for_update, current_version, download_and_verify, is_installed_copy,
    update_config,
};

use crate::app::Ctx;
use crate::platform::open_url::open_url;
use crate::platform::window_ext;
use crate::vm::error::error_code;
use crate::{MainWindow, SettingsBridge, ToastKind, Tr};

/// `SettingsBridge.update-state` values.
const UPDATE_IDLE: i32 = 0;
const UPDATE_CHECKING: i32 = 1;
const UPDATE_LATEST: i32 = 2;
const UPDATE_AVAILABLE: i32 = 3;
const UPDATE_FAILED: i32 = 4;
const UPDATE_DOWNLOADING: i32 = 5;

/// How often the quiet check runs again while the app stays open.
pub const RECHECK: Duration = Duration::from_secs(24 * 60 * 60);

/// Wait before the single retry of a failed start-up check (the network may not be up yet
/// right after sign-in).
pub const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

/// A found update is toasted at once only while the main window can be seen; otherwise the
/// toast would expire unseen and the notice waits for the next time the window is shown.
pub fn toast_now(window_visible: bool) -> bool {
    window_visible
}

/// Which check runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    /// The button: always reports the result.
    Manual,
    /// The one at start: quiet, and a failure schedules one retry.
    Startup,
    /// The daily one and the retry: quiet.
    Quiet,
}

thread_local! {
    /// The controller, for the event-loop continuations of the worker thread (a `Weak<Rc>` is
    /// not `Send`, so they cannot capture it).
    static CONTROLLER: RefCell<Weak<UpdatesController>> = const { RefCell::new(Weak::new()) };
}

/// A check or a download is under way: no second one may start.
pub fn busy(state: i32) -> bool {
    state == UPDATE_CHECKING || state == UPDATE_DOWNLOADING
}

/// The quiet check is skipped while a found update waits: a failed check would hide it and
/// the "new version" toast would repeat every day. A manual check always runs.
pub fn quiet_check_wanted(state: i32) -> bool {
    state != UPDATE_AVAILABLE
}

/// Fixed name: the manifest is remote data and must not pick a path.
pub fn installer_path() -> PathBuf {
    std::env::temp_dir().join("TimeTracker-update.exe")
}

/// Removes the installer left by an earlier run, and a half-written download. Errors are
/// ignored: usually nothing is there.
pub fn cleanup_installer(path: &Path) {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(PathBuf::from(part));
}

pub struct UpdatesController {
    ctx: Ctx,
    ui: slint::Weak<MainWindow>,
    /// The newer release found by the last update check.
    update: Arc<Mutex<Option<Manifest>>>,
    /// Runs the daily check; kept here because a dropped timer stops.
    timer: slint::Timer,
    /// The pending retry of a failed start-up check; restarting it cannot stack retries.
    retry: slint::Timer,
    /// A quiet check found an update while the main window was hidden: toast on next show.
    notice_pending: Cell<bool>,
}

/// Wires the update callbacks of `SettingsBridge` and starts the quiet checks.
pub fn install(ui: &MainWindow, ctx: &Ctx) -> Rc<UpdatesController> {
    cleanup_installer(&installer_path());
    let ctl = Rc::new(UpdatesController {
        ctx: ctx.clone(),
        ui: ui.as_weak(),
        update: Arc::default(),
        timer: slint::Timer::default(),
        retry: slint::Timer::default(),
        notice_pending: Cell::new(false),
    });
    if update_config().is_none() {
        return ctl;
    }
    let b = ui.global::<SettingsBridge>();
    b.set_updates_enabled(true);
    b.set_update_installable(std::env::current_exe().is_ok_and(|exe| is_installed_copy(&exe)));
    let c = ctl.clone();
    b.on_check_updates(move || c.check_updates(Check::Manual));
    let c = ctl.clone();
    b.on_install_update(move || c.install_update());
    let c = ctl.clone();
    b.on_open_update_page(move || c.open_update_page());

    CONTROLLER.with(|c| *c.borrow_mut() = Rc::downgrade(&ctl));
    let weak = Rc::downgrade(&ctl);
    window_ext::on_main_shown(move || {
        if let Some(c) = weak.upgrade() {
            c.show_pending_notice();
        }
    });

    ctl.check_quietly(Check::Startup);
    let weak = Rc::downgrade(&ctl);
    ctl.timer
        .start(slint::TimerMode::Repeated, RECHECK, move || {
            if let Some(c) = weak.upgrade() {
                c.check_quietly(Check::Quiet);
            }
        });
    ctl
}

impl UpdatesController {
    fn set_update_state(&self, state: i32, version: &str) {
        if let Some(ui) = self.ui.upgrade() {
            let b = ui.global::<SettingsBridge>();
            b.set_update_state(state);
            b.set_update_version(version.into());
        }
    }

    /// A check or a download is already running (or the window is gone).
    fn is_busy(&self) -> bool {
        self.ui
            .upgrade()
            .is_none_or(|ui| busy(ui.global::<SettingsBridge>().get_update_state()))
    }

    /// The check at start, every `RECHECK` and the retry, if the user wants it at that moment.
    fn check_quietly(&self, kind: Check) {
        let wanted = self.ctx.borrow().settings.update_auto_check
            && self.ui.upgrade().is_some_and(|ui| {
                quiet_check_wanted(ui.global::<SettingsBridge>().get_update_state())
            });
        if wanted {
            self.check_updates(kind);
        }
    }

    /// Shows the "new version" toast that was held back while the window was hidden.
    fn show_pending_notice(&self) {
        if !self.notice_pending.replace(false) {
            return;
        }
        if let Some(ui) = self.ui.upgrade() {
            let text = ui.global::<Tr>().get_update_available();
            ui.invoke_show_toast(text, ToastKind::Info);
        }
    }

    /// Runs one more quiet check after `RETRY_AFTER`, replacing a retry that still waits.
    fn schedule_retry(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.retry
            .start(slint::TimerMode::SingleShot, RETRY_AFTER, move || {
                if let Some(c) = weak.upgrade() {
                    c.check_quietly(Check::Quiet);
                }
            });
    }

    /// Looks for a newer release on a worker thread. A quiet check reports only a found
    /// update.
    fn check_updates(&self, kind: Check) {
        let quiet = kind != Check::Manual;
        let Some(cfg) = update_config() else { return };
        if self.is_busy() {
            return;
        }
        self.set_update_state(UPDATE_CHECKING, "");
        let (ui, slot) = (self.ui.clone(), self.update.clone());
        std::thread::spawn(move || {
            let found = check_for_update(&cfg, current_version()).map_err(|e| {
                log::warn!("update check failed: {}", error_code(&e));
            });
            if let Ok(m) = &found {
                *slot.lock().unwrap_or_else(|p| p.into_inner()) = m.clone();
            }
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui.upgrade() else { return };
                if kind == Check::Startup && found.is_err() {
                    CONTROLLER.with(|c| {
                        if let Some(c) = c.borrow().upgrade() {
                            c.schedule_retry();
                        }
                    });
                }
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
                if quiet && state == UPDATE_AVAILABLE {
                    let w = ui.window();
                    if !toast_now(w.is_visible() && !w.is_minimized()) {
                        CONTROLLER.with(|c| {
                            if let Some(c) = c.borrow().upgrade() {
                                c.notice_pending.set(true);
                            }
                        });
                        return;
                    }
                }
                ui.invoke_show_toast(text, kind);
            });
        });
    }

    /// Opens the release page of the found version in the browser.
    fn open_update_page(&self) {
        let page = self
            .update
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|m| m.page.clone());
        let Some(page) = page else { return };
        if open_url(&page).is_err()
            && let Some(ui) = self.ui.upgrade()
        {
            let text = ui.global::<Tr>().get_err_generic();
            ui.invoke_show_toast(text, ToastKind::Error);
        }
    }

    /// Downloads the found installer to the temp folder (the file appears only when its
    /// checksum matches the one in the signed manifest), starts it silently and quits so
    /// that it can replace the program.
    fn install_update(&self) {
        if update_config().is_none() || self.is_busy() {
            return;
        }
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
        let dest = installer_path();
        std::thread::spawn(move || {
            let res = download_and_verify(&manifest, &dest).map_err(|e| {
                log::warn!("update download failed: {}", error_code(&e));
            });
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ui.upgrade() else { return };
                let started = res
                    .and_then(|()| {
                        std::process::Command::new(&dest)
                            .args(["/S", "/UPDATE"])
                            .spawn()
                            .map_err(|_| ())
                    })
                    .is_ok();
                if started {
                    let _ = slint::quit_event_loop();
                } else {
                    ui.global::<SettingsBridge>()
                        .set_update_state(UPDATE_AVAILABLE);
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
    fn busy_states_block_new_work() {
        assert!(busy(UPDATE_CHECKING) && busy(UPDATE_DOWNLOADING));
        for s in [UPDATE_IDLE, UPDATE_LATEST, UPDATE_AVAILABLE, UPDATE_FAILED] {
            assert!(!busy(s));
        }
    }

    #[test]
    fn quiet_check_waits_while_update_is_found() {
        assert!(!quiet_check_wanted(UPDATE_AVAILABLE));
        for s in [UPDATE_IDLE, UPDATE_LATEST, UPDATE_FAILED] {
            assert!(quiet_check_wanted(s));
        }
    }

    #[test]
    fn found_update_is_toasted_only_while_the_window_is_visible() {
        assert!(toast_now(true));
        assert!(!toast_now(false));
    }

    #[test]
    fn cleanup_removes_installer_and_part() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("TimeTracker-update.exe");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::write(dir.path().join("TimeTracker-update.exe.part"), b"x").unwrap();
        cleanup_installer(&exe);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        cleanup_installer(&exe); // nothing there: no panic
    }
}
