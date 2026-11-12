// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context;
use slint::ComponentHandle;
use tt_app::app::{self, AppContext, Ctx, InitialWindow};
use tt_app::controllers::{
    calendar, catalog, dashboard, entries, mini, pickers, reports, settings, timer, tray,
};
use tt_app::platform::{autostart, dock, fatal, single_instance, tray_host, window_ext};
use tt_app::{MainWindow, logging};

fn main() -> anyhow::Result<()> {
    destroy_windows_on_hide();
    force_x11();
    logging::init();
    let result = run();
    if let Err(e) = &result {
        // Outermost context only (fixed text, no paths).
        log::error!("startup failed: {e}");
        if cfg!(not(debug_assertions)) {
            // `Tr` may not exist yet, so this one message is English.
            fatal::show_error(&format!("TimeTracker could not start ({e})."));
        }
    }
    result
}

/// Makes Slint's winit backend destroy a window when it is hidden (and create it again on
/// `show()`). A window hidden in the tray otherwise keeps its native window and frame
/// buffer and is still repainted on every 1 s timer tick (docs/perf.md).
#[allow(unsafe_code)]
fn destroy_windows_on_hide() {
    // SAFETY: the first statement of `main`, before any other thread exists.
    unsafe { std::env::set_var("SLINT_DESTROY_WINDOW_ON_HIDE", "1") };
}

/// Linux: always run through X11 (XWayland on Wayland sessions). winit 0.30 picks Wayland
/// only when `WAYLAND_DISPLAY` is set, and Wayland forbids always-on-top and window
/// positioning, which the mini window needs (spec section 5.2).
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn force_x11() {
    // SAFETY: called right after `destroy_windows_on_hide`, before any other thread exists.
    unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
}

#[cfg(not(target_os = "linux"))]
fn force_x11() {}

fn run() -> anyhow::Result<()> {
    // A second launch only activates the running instance and exits with code 0.
    let Some(mut instance) = single_instance::acquire()? else {
        log::info!("already running, activated the other instance");
        return Ok(());
    };
    let minimized = autostart::start_minimized(std::env::args());

    // A test database (`--db`) leaves the real autostart entry alone.
    let opened = match app::db_override(std::env::args_os()) {
        Some(path) => AppContext::open(path)?,
        None => AppContext::open_default()?,
    };
    let ctx: Ctx = Rc::new(RefCell::new(opened));
    let ui = MainWindow::new().context("create main window")?;
    if let Err(e) = slint::set_xdg_app_id(window_ext::APP_ID) {
        log::warn!("app id not set: {e}");
    }

    let lang = ctx
        .borrow_mut()
        .resolve_language(sys_locale::get_locale().as_deref())?;
    app::apply_language(lang);
    app::apply_theme(&ui, ctx.borrow().settings.theme);

    pickers::install(&ui);
    calendar::install(&ui, &ctx);
    let timer = timer::install(&ui, &ctx);
    let _entries = entries::install(&ui, &ctx, &timer);
    let _catalog = catalog::install(&ui, &ctx, &timer);
    let _dashboard = dashboard::install(&ui, &ctx, &timer);
    let _reports = reports::install(&ui, &ctx);

    // A login start waits a while for the tray host (it may start after us).
    let tray_host = tray_host::available(if minimized {
        tray_host::TRAY_WAIT_AT_LOGIN
    } else {
        Duration::ZERO
    });

    // The Run entry follows the setting, unless Task Manager disabled it (the user's
    // choice: then the setting turns off). Before the settings screen reads it.
    if ctx.borrow().manages_autostart {
        sync_autostart(&ctx);
    }

    // The mini timer is shown here when the settings say so, before the main window.
    let mini = mini::install(&ui, &ctx, &timer);
    #[cfg(not(target_os = "macos"))]
    let tray = if tray_host {
        tray::install(&ui, &ctx, &timer, &mini)
    } else {
        None
    };
    // The settings screen needs to know whether a tray exists. On macOS the menu bar icon
    // comes once the event loop runs (`install_menu_bar`): until then, the expected one.
    #[cfg(not(target_os = "macos"))]
    let has_tray = tray.is_some();
    #[cfg(target_os = "macos")]
    let has_tray = tray_host;
    ctx.borrow_mut().has_tray = has_tray;
    let _settings = settings::install(&ui, &ctx, &timer, &mini);

    // The close button hides the window to the tray (the menu bar on macOS; setting "close
    // to tray", needs a tray), else quits. A hidden main window takes the Dock icon along.
    let close_ctx = ctx.clone();
    ui.window().on_close_requested(move || {
        let c = close_ctx.borrow();
        // The tray host is probed again: it may have gone since start-up, and a hidden
        // window without an icon would leave the timer unreachable.
        if c.settings.close_to_tray && c.has_tray && tray_host::host_present_now() {
            dock::set_visible(false);
        } else {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });

    // Never leave the timer invisible: a `--minimized` start without a tray shows a
    // minimized window instead.
    let initial = app::initial_window(minimized, has_tray);
    match initial {
        InitialWindow::Shown => {
            ui.show().context("show main window")?;
            window_ext::when_created(&ui, |ui: &MainWindow| {
                window_ext::set_app_icon(ui.window());
            });
        }
        InitialWindow::Hidden => {}
        InitialWindow::Minimized => {
            ui.show().context("show main window")?;
            window_ext::when_created(&ui, |ui: &MainWindow| {
                window_ext::set_app_icon(ui.window());
                ui.window().set_minimized(true);
            });
        }
    }
    #[cfg(target_os = "macos")]
    let _menu_bar = install_menu_bar(
        &ui,
        &ctx,
        &timer,
        &mini,
        tray_host,
        initial == InitialWindow::Hidden,
    );
    let main = ui.as_weak();
    instance.on_activate(move || {
        if let Some(ui) = main.upgrade() {
            window_ext::show_and_focus(&ui);
        }
    });

    slint::run_event_loop_until_quit().context("event loop")?;
    mini.save_position();
    Ok(())
}

/// macOS: the menu bar icon and the reopen handler, once the event loop runs (`tray-icon`
/// needs a running event loop there, and `NSApplication` sets its own reopen handler while
/// it launches). Keep the returned slot until the loop returns: it holds the icon.
#[cfg(target_os = "macos")]
fn install_menu_bar(
    ui: &MainWindow,
    ctx: &Ctx,
    timer: &Rc<timer::TimerHub>,
    mini: &Rc<mini::MiniController>,
    tray_host: bool,
    hidden: bool,
) -> Rc<RefCell<Option<Rc<tt_app::platform::tray::Tray>>>> {
    let slot = Rc::new(RefCell::new(None));
    let (main, ctx, timer, mini, out) = (
        ui.as_weak(),
        ctx.clone(),
        timer.clone(),
        mini.clone(),
        slot.clone(),
    );
    slint::Timer::single_shot(Duration::ZERO, move || {
        let reopen = main.clone();
        dock::on_reopen(move || {
            if let Some(ui) = reopen.upgrade() {
                window_ext::show_and_focus(&ui);
            }
        });
        let Some(ui) = main.upgrade() else { return };
        let tray = if tray_host {
            tray::install(&ui, &ctx, &timer, &mini)
        } else {
            None
        };
        match tray {
            Some(t) => {
                out.replace(Some(t));
                // Only now: winit makes an unbundled binary a regular app while it launches.
                // A reopen or an activation may have shown the window since: keep its icon.
                if hidden && !ui.window().is_visible() {
                    dock::set_visible(false);
                }
            }
            // Never leave the timer invisible: no icon, so the main window and the Dock icon.
            None => {
                ctx.borrow_mut().has_tray = false;
                ui.global::<tt_app::SettingsBridge>()
                    .set_tray_available(false);
                window_ext::show_and_focus(&ui);
            }
        }
    });
    slot
}

fn sync_autostart(ctx: &Ctx) {
    let wanted = ctx.borrow().settings.autostart;
    match autostart::sync(wanted) {
        Ok(effective) if effective != wanted => {
            let mut c = ctx.borrow_mut();
            c.settings.autostart = effective;
            if let Err(e) = c.save_settings() {
                log::warn!("autostart setting not saved: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => log::warn!("autostart not updated: {e:#}"),
    }
}
