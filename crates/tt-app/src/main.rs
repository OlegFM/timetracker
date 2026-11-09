// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Context;
use slint::ComponentHandle;
use tt_app::app::{self, AppContext, Ctx};
use tt_app::controllers::{pickers, timer};
use tt_app::{MainWindow, logging};

fn main() -> anyhow::Result<()> {
    logging::init();
    let result = run();
    if let Err(e) = &result {
        // Outermost context only (fixed text, no paths).
        log::error!("startup failed: {e}");
    }
    result
}

fn run() -> anyhow::Result<()> {
    let ctx: Ctx = Rc::new(RefCell::new(AppContext::open_default()?));
    let ui = MainWindow::new().context("create main window")?;

    let lang = ctx
        .borrow_mut()
        .resolve_language(sys_locale::get_locale().as_deref())?;
    app::apply_language(lang);
    app::apply_theme(&ui, ctx.borrow().settings.theme);

    pickers::install(&ui);
    // Kept for the mini timer (Task 13), which attaches its window to the same hub.
    let _timer = timer::install(&ui, &ctx);

    // Until close-to-tray exists (Task 14), closing the window quits.
    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });

    ui.show().context("show main window")?;
    slint::run_event_loop_until_quit().context("event loop")?;
    Ok(())
}
