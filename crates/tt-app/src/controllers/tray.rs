//! Tray icon wiring: follows the [`TimerHub`] view and the mini timer visibility, and runs
//! the tray's [`AppAction`]s on the timer, the mini timer and the main window.

use std::rc::{Rc, Weak};

use anyhow::Context;
use slint::ComponentHandle;

use crate::controllers::mini::MiniController;
use crate::controllers::timer::TimerHub;
use crate::platform::tray::{AppAction, Tray, TrayState, TrayStrings, tray_label};
use crate::platform::window_ext;
use crate::{MainWindow, Tr};

/// Creates the tray icon (keep it for the app's lifetime). Call before the event loop runs.
pub fn install(
    ui: &MainWindow,
    hub: &Rc<TimerHub>,
    mini: &Rc<MiniController>,
) -> anyhow::Result<Rc<Tray>> {
    let (main, h, m) = (ui.as_weak(), Rc::downgrade(hub), Rc::downgrade(mini));
    let tray = Rc::new(
        Tray::new(strings(&ui.global::<Tr>()), move |a| act(a, &main, &h, &m))
            .context("create tray icon")?,
    );

    let t = Rc::downgrade(&tray);
    mini.on_visibility_changed(move |on| {
        if let Some(t) = t.upgrade() {
            t.set_mini_visible(on);
        }
    });
    tray.set_mini_visible(mini.is_visible());

    let t = Rc::downgrade(&tray);
    hub.on_view(move |v| {
        if let Some(t) = t.upgrade() {
            let label = tray_label(&v.description, &v.project_label);
            t.set_state(TrayState::from_view(v.state), label, &v.clock);
        }
    });
    Ok(tray)
}

/// The tray texts in the current language (also for `Tray::refresh_strings`).
pub fn strings(tr: &Tr<'_>) -> TrayStrings {
    TrayStrings {
        pause: tr.get_tray_pause().into(),
        resume: tr.get_tray_resume().into(),
        stop: tr.get_tray_stop().into(),
        show_mini: tr.get_tray_show_mini().into(),
        hide_mini: tr.get_tray_hide_mini().into(),
        open: tr.get_tray_open().into(),
        quit: tr.get_tray_quit().into(),
        no_active_task: tr.get_no_active_task().into(),
    }
}

fn act(
    action: AppAction,
    main: &slint::Weak<MainWindow>,
    hub: &Weak<TimerHub>,
    mini: &Weak<MiniController>,
) {
    let on_hub = |f: fn(&TimerHub)| {
        if let Some(h) = hub.upgrade() {
            f(&h);
        }
    };
    match action {
        AppAction::Pause => on_hub(TimerHub::pause),
        AppAction::Resume => on_hub(TimerHub::resume),
        AppAction::Stop => on_hub(TimerHub::stop),
        AppAction::ToggleMini => {
            if let Some(m) = mini.upgrade() {
                m.set_visible(!m.is_visible());
            }
        }
        AppAction::OpenMain => {
            if let Some(ui) = main.upgrade() {
                window_ext::show_and_focus(&ui);
            }
        }
        AppAction::ToggleMain => {
            let Some(ui) = main.upgrade() else { return };
            let w = ui.window();
            if w.is_visible() && !w.is_minimized() {
                if let Err(e) = ui.hide() {
                    log::warn!("main window not hidden: {e}");
                }
            } else {
                window_ext::show_and_focus(&ui);
            }
        }
        // The running session is left as it is; `main` saves the mini timer position after
        // the event loop returns.
        AppAction::Quit => {
            if let Err(e) = slint::quit_event_loop() {
                log::warn!("quit failed: {e}");
            }
        }
    }
}
