// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Context;
use slint::ComponentHandle;
use tt_app::app::{self, AppContext, Ctx};
use tt_app::controllers::{
    calendar, catalog, dashboard, entries, mini, pickers, reports, settings, timer, tray,
};
use tt_app::platform::{autostart, fatal, single_instance, window_ext};
use tt_app::{MainWindow, logging};

fn main() -> anyhow::Result<()> {
    destroy_windows_on_hide();
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

fn run() -> anyhow::Result<()> {
    // A second launch only activates the running instance and exits with code 0.
    let Some(mut instance) = single_instance::acquire()? else {
        log::info!("already running, activated the other instance");
        return Ok(());
    };
    let minimized = autostart::start_minimized(std::env::args());

    // A test database (`--db`) leaves the real autostart entry alone.
    let db_override = app::db_override(std::env::args_os());
    let opened = match &db_override {
        Some(path) => AppContext::open(path.clone())?,
        None => AppContext::open_default()?,
    };
    let ctx: Ctx = Rc::new(RefCell::new(opened));
    let ui = MainWindow::new().context("create main window")?;

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

    // ✕ hides the window to the tray (setting "close to tray"), else quits.
    let close_ctx = ctx.clone();
    ui.window().on_close_requested(move || {
        if !close_ctx.borrow().settings.close_to_tray {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });

    if !minimized {
        ui.show().context("show main window")?;
    }
    // The Run entry follows the setting, unless Task Manager disabled it (the user's
    // choice: then the setting turns off).
    if db_override.is_none() {
        sync_autostart(&ctx);
    }

    // Shown after the main window when the settings say so.
    let mini = mini::install(&ui, &ctx, &timer);
    let _settings = settings::install(&ui, &ctx, &timer, &mini);
    let _tray = tray::install(&ui, &ctx, &timer, &mini)?;

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
