//! Floating mini timer window: created on first show and kept (hidden) afterwards; its
//! `TimerBridge` is attached to the [`TimerHub`]. Visibility and position are saved in the
//! settings (`mini_timer_visible`, `mini_timer_pos`, physical pixels).
//!
//! The OS moves the window during a drag and reports no end of it, so after a drag starts
//! the position is polled until it stays put, then saved (also when hiding and on exit).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::{self, Ctx};
use crate::controllers::timer::TimerHub;
use crate::platform::window_ext;
use crate::{MainWindow, MiniBridge, MiniTimer};

const POLL: Duration = Duration::from_millis(250);
/// Polls with an unchanged position before it is saved (≈ 500 ms).
const STABLE_POLLS: u32 = 2;

type VisibleListener = Box<dyn Fn(bool)>;

pub struct MiniController {
    ctx: Ctx,
    hub: Rc<TimerHub>,
    main: slint::Weak<MainWindow>,
    me: Weak<MiniController>,
    window: RefCell<Option<MiniTimer>>,
    /// Runs after a drag started, until the position is saved.
    poll: Timer,
    last_pos: Cell<Option<(i32, i32)>>,
    stable_polls: Cell<u32>,
    /// Told about every [`MiniController::set_visible`] (the tray menu text).
    on_visible: RefCell<Option<VisibleListener>>,
}

/// Connects the timer bar toggle of `main` and shows the mini timer if the settings say so.
pub fn install(main: &MainWindow, ctx: &Ctx, hub: &Rc<TimerHub>) -> Rc<MiniController> {
    let mini = Rc::new_cyclic(|me: &Weak<MiniController>| {
        let me2 = me.clone();
        main.global::<MiniBridge>().on_set_visible(move |on| {
            if let Some(m) = me2.upgrade() {
                m.set_visible(on);
            }
        });
        MiniController {
            ctx: ctx.clone(),
            hub: hub.clone(),
            main: main.as_weak(),
            me: me.clone(),
            window: RefCell::default(),
            poll: Timer::default(),
            last_pos: Cell::default(),
            stable_polls: Cell::default(),
            on_visible: RefCell::default(),
        }
    });
    let visible = ctx.borrow().settings.mini_timer_visible;
    mini.set_visible(visible);
    mini
}

impl MiniController {
    /// Shows or hides the mini timer and saves the choice (timer bar, tray, settings).
    pub fn set_visible(&self, on: bool) {
        if on {
            self.show();
        } else {
            self.hide();
        }
        if let Some(ui) = self.main.upgrade() {
            ui.global::<MiniBridge>().set_visible(on);
        }
        {
            let mut c = self.ctx.borrow_mut();
            if c.settings.mini_timer_visible != on {
                c.settings.mini_timer_visible = on;
                if let Err(e) = c.save_settings() {
                    log::warn!("mini timer visibility not saved: {e}");
                }
            }
        }
        if let Some(f) = self.on_visible.borrow().as_ref() {
            f(on);
        }
    }

    /// `f` runs after every [`MiniController::set_visible`] (replaces an earlier one).
    pub fn on_visibility_changed(&self, f: impl Fn(bool) + 'static) {
        self.on_visible.replace(Some(Box::new(f)));
    }

    pub fn is_visible(&self) -> bool {
        self.window
            .borrow()
            .as_ref()
            .is_some_and(|w| w.window().is_visible())
    }

    /// Saves the current position if the window is shown (also called on exit).
    pub fn save_position(&self) {
        let pos = self.window.borrow().as_ref().and_then(|w| {
            let win = w.window();
            (win.is_visible() && !win.is_minimized()).then(|| window_ext::position(win))
        });
        let Some(pos) = pos else { return };
        let mut c = self.ctx.borrow_mut();
        if c.settings.mini_timer_pos != Some(pos) {
            c.settings.mini_timer_pos = Some(pos);
            if let Err(e) = c.save_settings() {
                log::warn!("mini timer position not saved: {e}");
            }
        }
    }

    fn show(&self) {
        if self.window.borrow().is_none() {
            let Some(w) = self.create() else { return };
            self.window.replace(Some(w));
        }
        let saved = self.ctx.borrow().settings.mini_timer_pos;
        let window = self.window.borrow();
        let Some(w) = window.as_ref() else { return };
        // Created there; moved onto a monitor below if that point is on none.
        if let Some((x, y)) = saved {
            w.window().set_position(slint::PhysicalPosition::new(x, y));
        }
        if let Err(e) = w.show() {
            log::warn!("mini timer not shown: {e}");
            return;
        }
        window_ext::when_created(w, move |w: &MiniTimer| {
            let win = w.window();
            window_ext::hide_from_taskbar(win);
            window_ext::set_position(win, saved.unwrap_or((i32::MIN, i32::MIN)));
        });
    }

    fn hide(&self) {
        self.save_position();
        self.poll.stop();
        if let Some(w) = self.window.borrow().as_ref()
            && let Err(e) = w.hide()
        {
            log::warn!("mini timer not hidden: {e}");
        }
    }

    fn create(&self) -> Option<MiniTimer> {
        let w = match MiniTimer::new() {
            Ok(w) => w,
            Err(e) => {
                log::warn!("mini timer not created: {e}");
                return None;
            }
        };
        let (theme, opacity) = {
            let s = &self.ctx.borrow().settings;
            (s.theme, s.mini_timer_idle_opacity)
        };
        app::apply_theme(&w, theme);
        w.set_idle_opacity(opacity);
        self.hub.attach(&w);

        let me = self.me.clone();
        w.on_open_main(move || {
            if let Some(m) = me.upgrade() {
                m.open_main();
            }
        });
        let me = self.me.clone();
        w.on_drag_started(move || {
            if let Some(m) = me.upgrade() {
                m.drag_started();
            }
        });
        // Alt+F4 hides it like the toggle does.
        let me = self.me.clone();
        w.window().on_close_requested(move || {
            if let Some(m) = me.upgrade() {
                m.set_visible(false);
            }
            slint::CloseRequestResponse::HideWindow
        });
        Some(w)
    }

    fn open_main(&self) {
        if let Some(ui) = self.main.upgrade() {
            window_ext::show_and_focus(&ui);
        }
    }

    fn drag_started(&self) {
        if let Some(w) = self.window.borrow().as_ref() {
            window_ext::start_drag(w.window());
        }
        self.last_pos.set(None);
        self.stable_polls.set(0);
        let me = self.me.clone();
        self.poll.start(TimerMode::Repeated, POLL, move || {
            if let Some(m) = me.upgrade() {
                m.poll_position();
            }
        });
    }

    fn poll_position(&self) {
        let pos = self
            .window
            .borrow()
            .as_ref()
            .map(|w| window_ext::position(w.window()));
        if pos.is_some() && self.last_pos.replace(pos) == pos {
            self.stable_polls.set(self.stable_polls.get() + 1);
        } else {
            self.stable_polls.set(0);
        }
        if pos.is_none() || self.stable_polls.get() >= STABLE_POLLS {
            self.poll.stop();
            self.save_position();
        }
    }
}

/// A monitor's bounds in physical pixels and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

/// Window width and the margin to the screen corner, in logical pixels.
const WIDTH: f64 = 240.0;
const MARGIN: f64 = 24.0;

/// `pos` (the window's top-left corner, physical pixels) if it lies on some monitor, else
/// the top-right corner of the first monitor (callers put the primary one first): the
/// window's right edge `MARGIN` from the monitor's right edge, its top `MARGIN` below the
/// monitor's top. The logical sizes are converted with that monitor's scale factor, so at
/// 100 % this is `(right - (240 + 24), top + 24)`. Without monitors `pos` is kept.
pub fn clamp_to_monitors(pos: (i32, i32), monitors: &[Rect]) -> (i32, i32) {
    let (x, y) = pos;
    let on = |m: &Rect| x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height;
    if monitors.iter().any(on) {
        return pos;
    }
    let Some(m) = monitors.first() else {
        return pos;
    };
    let px = |logical: f64| (logical * m.scale).round() as i32;
    (m.x + m.width - px(WIDTH + MARGIN), m.y + px(MARGIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32, scale: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn point_on_a_monitor_is_kept() {
        let monitors = [
            rect(0, 0, 1920, 1080, 1.0),
            rect(1920, -200, 2560, 1440, 1.0),
        ];
        assert_eq!(clamp_to_monitors((100, 200), &monitors), (100, 200));
        assert_eq!(clamp_to_monitors((0, 0), &monitors), (0, 0));
        // On the second monitor, above the first one's top edge.
        assert_eq!(clamp_to_monitors((3000, -150), &monitors), (3000, -150));
    }

    #[test]
    fn point_outside_all_monitors_goes_to_top_right_of_the_first() {
        let monitors = [rect(0, 0, 1920, 1080, 1.0), rect(-1280, 0, 1280, 1024, 1.0)];
        let corner = (1920 - (240 + 24), 24);
        assert_eq!(clamp_to_monitors((5000, 5000), &monitors), corner);
        assert_eq!(clamp_to_monitors((-2000, 100), &monitors), corner);
        // Right and bottom edges are exclusive.
        assert_eq!(clamp_to_monitors((1920, 10), &[monitors[0]]), corner);
        assert_eq!(clamp_to_monitors((10, 1080), &[monitors[0]]), corner);
    }

    #[test]
    fn corner_offsets_follow_the_monitor_scale() {
        // 150 %: the 240 px wide window is 360 physical pixels wide.
        let monitors = [rect(100, 50, 2880, 1620, 1.5)];
        assert_eq!(
            clamp_to_monitors((-1, -1), &monitors),
            (100 + 2880 - 396, 50 + 36)
        );
    }

    #[test]
    fn no_monitors_keeps_the_point() {
        assert_eq!(clamp_to_monitors((-5, 7), &[]), (-5, 7));
    }
}
