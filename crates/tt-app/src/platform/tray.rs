//! Notification-area (tray) icon with its context menu (crate `tray-icon` / `muda`).
//!
//! The icon shows the timer state (idle / running / paused, drawn in code), the tooltip and
//! the first, disabled menu item show the current task and session time. Updates are
//! throttled: nothing changes unless the state, the label or the displayed minute does, so
//! the 1 s timer tick never rebuilds the menu.
//!
//! Menu and icon events arrive through the crates' global handlers; they are forwarded with
//! `slint::invoke_from_event_loop` and turned into [`AppAction`]s for the handler given to
//! [`Tray::new`] (kept in a thread local of the UI thread while the tray exists).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyhow::Context;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Icon edge in pixels (Windows scales it to the tray size).
pub const ICON_SIZE: u32 = 32;
/// Longest label in the tooltip, in UTF-16 units (Windows caps tooltips at 127).
const LABEL_MAX: usize = 100;

const ID_PAUSE: &str = "pause-resume";
const ID_STOP: &str = "stop";
const ID_MINI: &str = "mini";
const ID_OPEN: &str = "open";
const ID_QUIT: &str = "quit";

/// macOS opens the menu on a left click on a menu bar icon (as other menu bar apps do); on
/// Windows and Linux the left click toggles the main window.
const MENU_ON_LEFT_CLICK: bool = cfg!(target_os = "macos");

/// What the user asked for from the tray (handled by the app on the UI thread).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAction {
    Pause,
    Resume,
    Stop,
    ToggleMini,
    OpenMain,
    /// Left click on the icon (not on macOS, where it opens the menu): show the main window,
    /// or hide it when it is shown.
    ToggleMain,
    Quit,
}

/// Timer state as shown by the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Stopped,
    Running,
    Paused,
}

impl TrayState {
    /// From `TimerView.state` (0 stopped, 1 running, 2 paused).
    pub fn from_view(state: i32) -> TrayState {
        match state {
            1 => TrayState::Running,
            2 => TrayState::Paused,
            _ => TrayState::Stopped,
        }
    }

    fn color(self) -> [u8; 3] {
        match self {
            TrayState::Stopped => [0x8b, 0x91, 0x9c],
            TrayState::Running => [0x22, 0xc5, 0x5e],
            TrayState::Paused => [0xf5, 0x9e, 0x0b],
        }
    }
}

/// Localized menu texts (from the main window's `Tr` global).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayStrings {
    pub pause: String,
    pub resume: String,
    pub stop: String,
    pub show_mini: String,
    pub hide_mini: String,
    pub open: String,
    pub quit: String,
    pub no_active_task: String,
}

/// The state, label and clock last shown, and the clock's minute (`minute_of`).
#[derive(Debug, Clone)]
struct Shown {
    state: TrayState,
    label: String,
    minute: String,
    clock: String,
}

pub struct Tray {
    icon: TrayIcon,
    icons: [Icon; 3],
    status: MenuItem,
    pause: MenuItem,
    stop: MenuItem,
    mini: MenuItem,
    open: MenuItem,
    quit: MenuItem,
    strings: RefCell<TrayStrings>,
    shown: RefCell<Shown>,
    mini_visible: Cell<bool>,
    /// Shared with the event router (decides between Pause and Resume).
    state: Rc<Cell<TrayState>>,
}

struct Router {
    state: Rc<Cell<TrayState>>,
    on_action: Box<dyn Fn(AppAction)>,
}

thread_local! {
    static ROUTER: RefCell<Option<Rc<Router>>> = const { RefCell::new(None) };
}

impl Tray {
    /// Creates the tray icon (stopped state); call on the UI thread before the event loop
    /// runs (on macOS: once it runs) and keep the value for the app's lifetime (dropping it
    /// removes the icon).
    pub fn new(
        strings: TrayStrings,
        on_action: impl Fn(AppAction) + 'static,
    ) -> anyhow::Result<Tray> {
        let icon = |s| Icon::from_rgba(icon_rgba(s), ICON_SIZE, ICON_SIZE).context("tray icon");
        // In `TrayState` order.
        let icons = [
            icon(TrayState::Stopped)?,
            icon(TrayState::Running)?,
            icon(TrayState::Paused)?,
        ];

        let status = MenuItem::new(menu_text(&strings.no_active_task), false, None);
        let pause = MenuItem::with_id(ID_PAUSE, menu_text(&strings.pause), false, None);
        let stop = MenuItem::with_id(ID_STOP, menu_text(&strings.stop), false, None);
        let mini = MenuItem::with_id(ID_MINI, menu_text(&strings.show_mini), true, None);
        let open = MenuItem::with_id(ID_OPEN, menu_text(&strings.open), true, None);
        let quit = MenuItem::with_id(ID_QUIT, menu_text(&strings.quit), true, None);
        let menu = Menu::with_items(&[
            &status,
            &pause,
            &stop,
            &PredefinedMenuItem::separator(),
            &mini,
            &PredefinedMenuItem::separator(),
            &open,
            &quit,
        ])
        .context("tray menu")?;

        let icon = TrayIconBuilder::new()
            .with_icon(icons[0].clone())
            .with_tooltip(&strings.no_active_task)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(MENU_ON_LEFT_CLICK)
            .build()
            .context("tray icon")?;

        let state = Rc::new(Cell::new(TrayState::Stopped));
        ROUTER.with(|r| {
            r.replace(Some(Rc::new(Router {
                state: state.clone(),
                on_action: Box::new(on_action),
            })))
        });
        MenuEvent::set_event_handler(Some(|e: MenuEvent| {
            let id = e.id.0;
            post(move |state| menu_action(&id, state));
        }));
        TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
            if let Some(action) = click_action(&e, MENU_ON_LEFT_CLICK) {
                post(move |_| Some(action));
            }
        }));

        Ok(Tray {
            icon,
            icons,
            status,
            pause,
            stop,
            mini,
            open,
            quit,
            strings: RefCell::new(strings),
            shown: RefCell::new(Shown {
                state: TrayState::Stopped,
                label: String::new(),
                minute: String::new(),
                clock: String::new(),
            }),
            mini_visible: Cell::new(false),
            state,
        })
    }

    /// Shows the timer state; does nothing unless the state, the label or the displayed
    /// minute of `clock` ("H:MM:SS") changed.
    pub fn set_state(&self, state: TrayState, label: &str, clock: &str) {
        let next = Shown {
            state,
            label: label.to_owned(),
            minute: minute_of(clock).to_owned(),
            clock: clock.to_owned(),
        };
        let prev = self.shown.borrow().clone();
        if (prev.state, &prev.label, &prev.minute) == (state, &next.label, &next.minute) {
            return;
        }
        if prev.state != state {
            let icon = &self.icons[state as usize];
            if let Err(e) = self.icon.set_icon(Some(icon.clone())) {
                log::warn!("tray icon not updated: {e}");
            }
        }
        self.state.set(state);
        self.apply(&next);
        self.shown.replace(next);
    }

    /// Updates the "Show/Hide mini timer" item.
    pub fn set_mini_visible(&self, on: bool) {
        self.mini_visible.set(on);
        let s = self.strings.borrow();
        self.mini
            .set_text(menu_text(if on { &s.hide_mini } else { &s.show_mini }));
    }

    /// Replaces all texts (after a language switch).
    pub fn refresh_strings(&self, strings: TrayStrings) {
        self.open.set_text(menu_text(&strings.open));
        self.quit.set_text(menu_text(&strings.quit));
        self.stop.set_text(menu_text(&strings.stop));
        self.strings.replace(strings);
        self.set_mini_visible(self.mini_visible.get());
        let shown = self.shown.borrow().clone();
        self.apply(&shown);
    }

    /// Writes tooltip and menu texts for `s` with the current strings.
    fn apply(&self, s: &Shown) {
        let strings = self.strings.borrow();
        let text = tooltip_text(s.state, &s.label, &s.clock, &strings.no_active_task);
        if let Err(e) = self.icon.set_tooltip(Some(&text)) {
            log::warn!("tray tooltip not updated: {e}");
        }
        self.status.set_text(menu_text(&text));
        let paused = s.state == TrayState::Paused;
        self.pause.set_text(menu_text(if paused {
            &strings.resume
        } else {
            &strings.pause
        }));
        let active = s.state != TrayState::Stopped;
        self.pause.set_enabled(active);
        self.stop.set_enabled(active);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        ROUTER.with(|r| r.replace(None));
    }
}

/// Runs `pick` with the tray state on the UI thread and hands its action to the app.
fn post(pick: impl FnOnce(TrayState) -> Option<AppAction> + Send + 'static) {
    let res = slint::invoke_from_event_loop(move || {
        let Some(router) = ROUTER.with(|r| r.borrow().clone()) else {
            return;
        };
        if let Some(action) = pick(router.state.get()) {
            (router.on_action)(action);
        }
    });
    if let Err(e) = res {
        log::warn!("tray event dropped: {e}");
    }
}

/// The action of the menu item `id`; the Pause/Resume item depends on `state`.
fn menu_action(id: &str, state: TrayState) -> Option<AppAction> {
    let active = state != TrayState::Stopped;
    match id {
        ID_PAUSE if state == TrayState::Running => Some(AppAction::Pause),
        ID_PAUSE if state == TrayState::Paused => Some(AppAction::Resume),
        ID_STOP if active => Some(AppAction::Stop),
        ID_MINI => Some(AppAction::ToggleMini),
        ID_OPEN => Some(AppAction::OpenMain),
        ID_QUIT => Some(AppAction::Quit),
        _ => None,
    }
}

/// Left button released on the icon → toggle the main window; nothing when the left click
/// opens the menu (`menu_on_left_click`, macOS).
fn click_action(e: &TrayIconEvent, menu_on_left_click: bool) -> Option<AppAction> {
    if menu_on_left_click {
        return None;
    }
    match e {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => Some(AppAction::ToggleMain),
        _ => None,
    }
}

/// Tooltip and first menu item: `no_active` when stopped, else `"<label> — <clock>"`
/// (only the clock without a label), the label cut to 100 UTF-16 units with "…".
pub fn tooltip_text(state: TrayState, label: &str, clock: &str, no_active: &str) -> String {
    if state == TrayState::Stopped {
        return no_active.to_owned();
    }
    if label.is_empty() {
        return clock.to_owned();
    }
    format!("{} — {clock}", cut(label, LABEL_MAX))
}

/// `s` if it fits in `max` UTF-16 units, else its longest prefix that fits with "…".
fn cut(s: &str, max: usize) -> String {
    if s.encode_utf16().count() <= max {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut units = 1; // the ellipsis
    for c in s.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// What the tray names the current entry: its description, else its project label.
pub fn tray_label<'a>(description: &'a str, project_label: &'a str) -> &'a str {
    if description.is_empty() {
        project_label
    } else {
        description
    }
}

/// `"1:24"` for `"1:24:07"` (the part that changes once a minute).
fn minute_of(clock: &str) -> &str {
    clock.rsplit_once(':').map_or(clock, |(minute, _)| minute)
}

/// Menu text with `&` shown literally (a single `&` marks a mnemonic on Windows).
fn menu_text(s: &str) -> String {
    s.replace('&', "&&")
}

/// RGBA pixels (row-major, `ICON_SIZE`²) of a filled, anti-aliased circle in the state's
/// color on a transparent background.
pub fn icon_rgba(state: TrayState) -> Vec<u8> {
    let [r, g, b] = state.color();
    let size = ICON_SIZE as f32;
    let (center, radius) = (size / 2.0, size / 2.0 - 2.0);
    let mut out = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            // Distance of the pixel center; coverage falls off over one pixel at the edge.
            let (dx, dy) = (x as f32 + 0.5 - center, y as f32 + 0.5 - center);
            let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            out.extend([r, g, b, (coverage * 255.0).round() as u8]);
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use tray_icon::dpi::PhysicalPosition;
    use tray_icon::{Rect, TrayIconId};

    use super::*;

    const NO: &str = "No active task";

    #[test]
    fn tooltip_when_stopped_is_no_active_task() {
        assert_eq!(
            tooltip_text(TrayState::Stopped, "Reports", "0:00:00", NO),
            NO
        );
    }

    #[test]
    fn tooltip_shows_label_and_clock_when_active() {
        assert_eq!(
            tooltip_text(TrayState::Running, "Reports page", "1:24:07", NO),
            "Reports page — 1:24:07"
        );
        assert_eq!(
            tooltip_text(TrayState::Paused, "Reports page", "0:05:00", NO),
            "Reports page — 0:05:00"
        );
        assert_eq!(
            tooltip_text(TrayState::Running, "", "0:00:09", NO),
            "0:00:09"
        );
    }

    #[test]
    fn tooltip_label_is_cut_to_100() {
        let long = "x".repeat(150);
        let text = tooltip_text(TrayState::Running, &long, "1:24:07", NO);
        let (label, clock) = text.split_once(" — ").unwrap();
        assert_eq!(label.chars().count(), 100);
        assert!(label.ends_with('…'));
        assert_eq!(clock, "1:24:07");
        // Exactly 100 is kept as is.
        let exact = "y".repeat(100);
        let text = tooltip_text(TrayState::Running, &exact, "0:00:01", NO);
        assert_eq!(text, format!("{exact} — 0:00:01"));
        // Characters outside the BMP count twice (UTF-16), and are never split.
        let emoji = "😀".repeat(80);
        let text = tooltip_text(TrayState::Running, &emoji, "0:00:01", NO);
        let label = text.split_once(" — ").unwrap().0;
        assert!(label.encode_utf16().count() <= 100);
        assert!(text.encode_utf16().count() <= 127);
    }

    #[test]
    fn label_prefers_description_then_project() {
        assert_eq!(tray_label("Reports", "Site › Design"), "Reports");
        assert_eq!(tray_label("", "Site › Design"), "Site › Design");
        assert_eq!(tray_label("", ""), "");
    }

    #[test]
    fn view_state_maps_to_tray_state() {
        assert_eq!(TrayState::from_view(0), TrayState::Stopped);
        assert_eq!(TrayState::from_view(1), TrayState::Running);
        assert_eq!(TrayState::from_view(2), TrayState::Paused);
    }

    #[test]
    fn minute_drops_the_seconds() {
        assert_eq!(minute_of("1:24:07"), "1:24");
        assert_eq!(minute_of("12:00:59"), "12:00");
        assert_eq!(minute_of(""), "");
    }

    #[test]
    fn menu_text_escapes_ampersands() {
        assert_eq!(menu_text("R&D & QA"), "R&&D && QA");
        assert_eq!(menu_text("plain"), "plain");
    }

    #[test]
    fn menu_items_map_to_actions() {
        use TrayState::*;
        assert_eq!(menu_action(ID_PAUSE, Running), Some(AppAction::Pause));
        assert_eq!(menu_action(ID_PAUSE, Paused), Some(AppAction::Resume));
        assert_eq!(menu_action(ID_PAUSE, Stopped), None);
        assert_eq!(menu_action(ID_STOP, Running), Some(AppAction::Stop));
        assert_eq!(menu_action(ID_STOP, Stopped), None);
        assert_eq!(menu_action(ID_MINI, Stopped), Some(AppAction::ToggleMini));
        assert_eq!(menu_action(ID_OPEN, Paused), Some(AppAction::OpenMain));
        assert_eq!(menu_action(ID_QUIT, Running), Some(AppAction::Quit));
        assert_eq!(menu_action("other", Running), None);
    }

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: TrayIconId::new("t"),
            position: PhysicalPosition::new(0.0, 0.0),
            rect: Rect::default(),
            button,
            button_state,
        }
    }

    #[test]
    fn left_button_up_toggles_the_main_window() {
        use MouseButton::*;
        use MouseButtonState::*;
        assert_eq!(
            click_action(&click(Left, Up), false),
            Some(AppAction::ToggleMain)
        );
        assert_eq!(click_action(&click(Left, Down), false), None);
        assert_eq!(click_action(&click(Right, Up), false), None);
    }

    #[test]
    fn click_on_macos_opens_the_menu_and_does_not_toggle_the_window() {
        use MouseButton::*;
        use MouseButtonState::*;
        assert_eq!(click_action(&click(Left, Up), true), None);
        assert_eq!(
            click_action(&click(Left, Up), false),
            Some(AppAction::ToggleMain)
        );
    }

    fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * ICON_SIZE + x) * 4) as usize;
        rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn icon_is_a_filled_circle_in_the_state_color() {
        for (state, rgb) in [
            (TrayState::Stopped, [0x8b, 0x91, 0x9c]),
            (TrayState::Running, [0x22, 0xc5, 0x5e]),
            (TrayState::Paused, [0xf5, 0x9e, 0x0b]),
        ] {
            let rgba = icon_rgba(state);
            assert_eq!(rgba.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
            let c = ICON_SIZE / 2;
            assert_eq!(
                pixel(&rgba, c, c),
                [rgb[0], rgb[1], rgb[2], 255],
                "{state:?}"
            );
            for (x, y) in [
                (0, 0),
                (ICON_SIZE - 1, 0),
                (0, ICON_SIZE - 1),
                (ICON_SIZE - 1, ICON_SIZE - 1),
            ] {
                assert_eq!(pixel(&rgba, x, y)[3], 0, "corner of {state:?}");
            }
            // The edge is anti-aliased: some pixels are partly transparent.
            assert!(rgba.chunks(4).any(|p| p[3] > 0 && p[3] < 255));
            Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).unwrap();
        }
    }
}
