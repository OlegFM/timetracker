//! Native window tweaks through winit (Slint's winit backend, `slint/unstable-winit-030`).
//!
//! Slint creates the native window only after `show()`, once the event loop runs; until
//! then (or with another backend) these functions do nothing. Use [`when_created`] to run
//! them as soon as the window exists. Positions are physical pixels.

use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, PhysicalPosition};

use crate::controllers::mini::{Rect, clamp_to_monitors};
use crate::platform::dock;

/// Application id: the Wayland `app_id` and X11 `WM_CLASS`. Matches `StartupWMClass` and
/// the name of the `.desktop` file.
pub const APP_ID: &str = "timetracker";

/// Window icon for Linux: raw RGBA, `APP_ICON_SIZE` x `APP_ICON_SIZE`, row by row
/// (made by `tools/gen_icon.py`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const APP_ICON_RGBA: &[u8] = include_bytes!("../../assets/app-64.rgba");
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const APP_ICON_SIZE: u32 = 64;

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

/// Keeps the window above all others. Call after every `show()`: hidden windows are
/// destroyed (`main.rs`) and Slint does not apply its cached level to the new one.
pub fn keep_on_top(w: &slint::Window) {
    w.with_winit_window(|win| win.set_window_level(winit::window::WindowLevel::AlwaysOnTop));
}

/// Puts the application icon (icon resource 1 of the executable, see `build.rs`) in the
/// title bar and on the taskbar button. Call after every `show()`, like [`keep_on_top`]:
/// a Slint `icon` would not come back on a re-created window either.
pub fn set_app_icon(w: &slint::Window) {
    w.with_winit_window(|_win| {
        #[cfg(target_os = "windows")]
        {
            use winit::dpi::PhysicalSize;
            use winit::platform::windows::{IconExtWindows, WindowExtWindows};
            use winit::window::Icon;

            let px = |logical: f64| {
                let n = (logical * _win.scale_factor()).round() as u32;
                Some(PhysicalSize::new(n, n))
            };
            match (
                Icon::from_resource(1, px(16.0)),
                Icon::from_resource(1, px(32.0)),
            ) {
                (Ok(small), Ok(big)) => {
                    _win.set_window_icon(Some(small));
                    _win.set_taskbar_icon(Some(big));
                }
                (Err(e), _) | (_, Err(e)) => log::warn!("window icon not set: {e}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            match winit::window::Icon::from_rgba(
                APP_ICON_RGBA.to_vec(),
                APP_ICON_SIZE,
                APP_ICON_SIZE,
            ) {
                Ok(icon) => _win.set_window_icon(Some(icon)),
                Err(e) => log::warn!("window icon not set: {e}"),
            }
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

/// Shows `ui` (restoring it if minimized or hidden) and brings it to the front. For the main
/// window only: it also shows the Dock icon on macOS, so `focus` activates a regular app.
pub fn show_and_focus<C: ComponentHandle + 'static>(ui: &C) {
    dock::set_visible(true);
    ui.window().set_minimized(false);
    if let Err(e) = ui.show() {
        log::warn!("window not shown: {e}");
        return;
    }
    when_created(ui, |ui: &C| {
        set_app_icon(ui.window());
        focus(ui.window());
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_app_icon_is_64px_rgba_with_transparent_corners() {
        assert_eq!(
            APP_ICON_RGBA.len(),
            (APP_ICON_SIZE * APP_ICON_SIZE * 4) as usize
        );
        let alpha = |x: u32, y: u32| APP_ICON_RGBA[((y * APP_ICON_SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha(32, 32), 255);
        assert_eq!(alpha(0, 0), 0);
    }
}
