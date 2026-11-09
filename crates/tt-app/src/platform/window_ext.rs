//! Native window tweaks through winit (Slint's winit backend, `slint/unstable-winit-030`).
//!
//! Slint creates the native window only after `show()`, once the event loop runs; until
//! then (or with another backend) these functions do nothing. Use [`when_created`] to run
//! them as soon as the window exists. Positions are physical pixels.

use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, PhysicalPosition};

use crate::controllers::mini::{Rect, clamp_to_monitors};

/// Runs `f` once the native window of `ui` exists (right away if it already does).
pub fn when_created<C: ComponentHandle + 'static>(ui: &C, f: impl FnOnce(&C) + 'static) {
    let weak = ui.as_weak();
    let task = slint::spawn_local(async move {
        let Some(ui) = weak.upgrade() else { return };
        // The returned `Arc` is dropped at once: holding it would keep a hidden window alive.
        if ui.window().winit_window().await.is_ok() {
            f(&ui);
        }
    });
    if let Err(e) = task {
        log::warn!("window setup not scheduled: {e}");
    }
}

/// Removes the window's taskbar button. Call after every `show()`: the window may be
/// re-created when shown again.
pub fn hide_from_taskbar(w: &slint::Window) {
    w.with_winit_window(|_win| {
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowExtWindows;
            _win.set_skip_taskbar(true);
        }
    });
}

/// Hands the window to the OS to be moved with the mouse; call while the left button is
/// down. The OS ends the move when the button is released (no event is sent for that).
pub fn start_drag(w: &slint::Window) {
    w.with_winit_window(|win| {
        if let Err(e) = win.drag_window() {
            log::warn!("window drag failed: {e}");
        }
    });
}

/// The window's top-left corner on the desktop.
pub fn position(w: &slint::Window) -> (i32, i32) {
    let p = w.position();
    (p.x, p.y)
}

/// Moves the window's top-left corner to `pos`, or — when `pos` is on no monitor — to the
/// top-right corner of the primary monitor ([`clamp_to_monitors`]). Returns the position used.
pub fn set_position(w: &slint::Window, pos: (i32, i32)) -> (i32, i32) {
    let monitors = w.with_winit_window(monitors).unwrap_or_default();
    let (x, y) = clamp_to_monitors(pos, &monitors);
    w.set_position(PhysicalPosition::new(x, y));
    (x, y)
}

/// Brings the window to the front and gives it the keyboard focus.
pub fn focus(w: &slint::Window) {
    w.with_winit_window(|win| win.focus_window());
}

/// All monitors, the primary one first.
fn monitors(win: &winit::window::Window) -> Vec<Rect> {
    let mut list: Vec<_> = win.available_monitors().collect();
    if let Some(primary) = win.primary_monitor()
        && let Some(i) = list.iter().position(|m| *m == primary)
    {
        list[..=i].rotate_right(1);
    }
    list.iter()
        .map(|m| {
            let (pos, size) = (m.position(), m.size());
            Rect {
                x: pos.x,
                y: pos.y,
                width: i32::try_from(size.width).unwrap_or(i32::MAX),
                height: i32::try_from(size.height).unwrap_or(i32::MAX),
                scale: m.scale_factor(),
            }
        })
        .collect()
}
