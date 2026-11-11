//! Application context owned by the UI thread, and app-wide UI settings helpers.

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Context;
use directories::BaseDirs;
use slint::{ComponentHandle, Global};
use tt_services::{
    CatalogService, EntryService, Settings, SettingsService, SystemClock, TimerService,
};
use tt_storage::Db;

use crate::Theme;
use crate::vm::lang;

/// Folder name under the user data folder (`%APPDATA%` / `%LOCALAPPDATA%` on Windows,
/// `~/.local/share` on Linux, `~/Library/Application Support` on macOS).
const APP_DIR: &str = "TimeTracker";
/// Command-line flag: open the database file named by the next argument instead of the
/// default one (measurements and manual checks on a seed database, see `examples/seed.rs`).
pub const DB_ARG: &str = "--db";

pub struct AppContext {
    pub db: Db,
    pub timer: TimerService<SystemClock>,
    pub catalog: CatalogService<SystemClock>,
    pub entries: EntryService<SystemClock>,
    pub settings: Settings,
    pub db_path: PathBuf,
    pub events: Rc<AppEvents>,
    /// Whether the autostart entry in the registry follows the setting. Only for the
    /// default database: with `--db` the "Start with Windows" toggle changes the setting
    /// alone and nothing writes the real entry.
    pub manages_autostart: bool,
    /// Whether the tray icon was created (it needs a tray host on Linux). Without it the
    /// close button quits and "close to tray" is unavailable.
    pub has_tray: bool,
}

/// Callbacks of one kind of event.
#[derive(Default)]
struct Listeners(RefCell<Vec<Rc<dyn Fn()>>>);

impl Listeners {
    fn add(&self, f: impl Fn() + 'static) {
        self.0.borrow_mut().push(Rc::new(f));
    }

    /// Calls the listeners registered so far; they may subscribe or emit again.
    fn emit(&self) {
        let listeners = self.0.borrow().clone();
        for f in listeners {
            f();
        }
    }
}

/// Notifications between controllers (UI thread). Clone the `Rc` out of the context
/// before emitting so that listeners can borrow the context.
#[derive(Default)]
pub struct AppEvents {
    entries_changed: Listeners,
    settings_changed: Listeners,
}

impl AppEvents {
    /// `f` runs after time entries changed (timer actions, midnight rollover, edits).
    pub fn on_entries_changed(&self, f: impl Fn() + 'static) {
        self.entries_changed.add(f);
    }

    pub fn emit_entries_changed(&self) {
        self.entries_changed.emit();
    }

    /// `f` runs after a setting changed (language, formats, time zone, theme, a restore):
    /// everything rendered from the settings is drawn again.
    pub fn on_settings_changed(&self, f: impl Fn() + 'static) {
        self.settings_changed.add(f);
    }

    pub fn emit_settings_changed(&self) {
        self.settings_changed.emit();
    }

    /// `f` runs after entries or settings changed (views that show both).
    pub fn on_data_changed(&self, f: impl Fn() + Clone + 'static) {
        self.on_entries_changed(f.clone());
        self.on_settings_changed(f);
    }
}

/// Shared handle used by controllers (UI thread only).
pub type Ctx = Rc<RefCell<AppContext>>;

impl AppContext {
    /// Opens (creating if needed) `timetracker.db` in the app folder of the user data
    /// directory (`%APPDATA%\TimeTracker` on Windows, `~/.local/share/TimeTracker` on Linux,
    /// `~/Library/Application Support/TimeTracker` on macOS).
    pub fn open_default() -> anyhow::Result<AppContext> {
        let dirs = BaseDirs::new().context("no user profile folders")?;
        let mut ctx = Self::open(dirs.data_dir().join(APP_DIR).join("timetracker.db"))?;
        ctx.manages_autostart = true;
        Ok(ctx)
    }

    /// Opens the database at `db_path` (creating its folder) and loads the settings. The
    /// autostart entry is left alone ([`Self::manages_autostart`]).
    pub fn open(db_path: PathBuf) -> anyhow::Result<AppContext> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir).context("create data folder")?;
        }
        let db = Db::open(&db_path).context("open database")?;
        let settings = SettingsService::load(&db).context("load settings")?;
        Ok(AppContext {
            db,
            timer: TimerService::new(SystemClock),
            catalog: CatalogService::new(SystemClock),
            entries: EntryService::new(SystemClock),
            settings,
            db_path,
            events: Rc::default(),
            manages_autostart: false,
            has_tray: false,
        })
    }

    pub fn save_settings(&mut self) -> anyhow::Result<()> {
        SettingsService::save(&mut self.db, &self.settings).context("save settings")
    }

    /// The UI language to use; a missing or unsupported saved value is replaced by the one
    /// derived from `os_locale` and saved.
    pub fn resolve_language(&mut self, os_locale: Option<&str>) -> anyhow::Result<&'static str> {
        let lang = lang::initial_language(&self.settings.language, os_locale);
        if self.settings.language != lang {
            self.settings.language = lang.to_owned();
            self.save_settings()?;
        }
        Ok(lang)
    }
}

/// The database chosen instead of the default one: the path after [`DB_ARG`] in `args`,
/// else (debug builds only) the `TT_DB_PATH` environment variable.
pub fn db_override(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    db_arg(args).or_else(env_db)
}

#[cfg(debug_assertions)]
fn env_db() -> Option<PathBuf> {
    std::env::var_os("TT_DB_PATH")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(debug_assertions))]
fn env_db() -> Option<PathBuf> {
    None
}

/// The argument after [`DB_ARG`], unless it is empty or another flag.
fn db_arg(args: impl IntoIterator<Item = OsString>) -> Option<PathBuf> {
    let mut args = args.into_iter();
    args.position(|a| a == DB_ARG)?;
    args.next()
        .filter(|p| !p.is_empty() && !p.to_string_lossy().starts_with("--"))
        .map(PathBuf::from)
}

/// The log folder: `%LOCALAPPDATA%\TimeTracker\logs` on Windows,
/// `~/.local/share/TimeTracker/logs` on Linux, `~/Library/Application Support/TimeTracker/logs`
/// on macOS.
pub fn log_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|d| d.data_local_dir().join(APP_DIR).join("logs"))
}

/// Switches all windows to the bundled catalog of `lang` (`en` = source strings).
/// Needs an existing window: the catalogs live in the Slint context.
pub fn apply_language(lang: &str) {
    if let Err(e) = slint::select_bundled_translation(lang) {
        log::warn!("language {lang} not selected: {e}");
    }
}

/// Sets the theme of one window (each window has its own `Theme` global).
pub fn apply_theme<C>(ui: &C, theme: tt_services::Theme)
where
    C: ComponentHandle,
    for<'a> Theme<'a>: Global<'a, C>,
{
    ui.global::<Theme>()
        .set_dark(theme == tt_services::Theme::Dark);
}

/// How the main window appears at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialWindow {
    Shown,
    /// Not shown: only the tray icon (login start).
    Hidden,
    /// Shown minimized: a login start without a tray must not leave the timer invisible.
    Minimized,
}

/// The first state of the main window for a `--minimized` start and a working tray.
pub fn initial_window(minimized: bool, tray: bool) -> InitialWindow {
    match (minimized, tray) {
        (false, _) => InitialWindow::Shown,
        (true, true) => InitialWindow::Hidden,
        (true, false) => InitialWindow::Minimized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_window_follows_minimized_and_tray() {
        use InitialWindow::*;
        assert_eq!(initial_window(false, true), Shown);
        assert_eq!(initial_window(false, false), Shown);
        assert_eq!(initial_window(true, true), Hidden);
        assert_eq!(initial_window(true, false), Minimized);
    }

    #[test]
    fn open_creates_folder_and_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("timetracker.db");
        let ctx = AppContext::open(path.clone()).unwrap();
        assert!(path.exists());
        assert_eq!(ctx.db_path, path);
        assert_eq!(ctx.settings, Settings::default());
        // Only the default database may write the autostart entry in the registry.
        assert!(!ctx.manages_autostart);
    }

    #[test]
    fn entries_changed_reaches_all_listeners_and_allows_reentry() {
        use std::cell::Cell;

        let events = Rc::new(AppEvents::default());
        let calls = Rc::new(Cell::new(0));
        for _ in 0..2 {
            let calls = calls.clone();
            events.on_entries_changed(move || calls.set(calls.get() + 1));
        }
        // A listener that subscribes another one while being called.
        let ev = events.clone();
        events.on_entries_changed(move || ev.on_entries_changed(|| {}));

        events.emit_entries_changed();
        assert_eq!(calls.get(), 2);
        events.emit_entries_changed();
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn settings_changed_is_separate_from_entries_changed() {
        use std::cell::Cell;

        let events = AppEvents::default();
        let (entries, settings, both) =
            (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Cell::new(0));
        let both = Rc::new(both);
        let e = entries.clone();
        events.on_entries_changed(move || e.set(e.get() + 1));
        let s = settings.clone();
        events.on_settings_changed(move || s.set(s.get() + 1));
        let b = both.clone();
        events.on_data_changed(move || b.set(b.get() + 1));

        events.emit_settings_changed();
        assert_eq!((entries.get(), settings.get(), both.get()), (0, 1, 1));
        events.emit_entries_changed();
        assert_eq!((entries.get(), settings.get(), both.get()), (1, 1, 2));
    }

    #[test]
    fn db_flag_takes_the_next_argument() {
        let args = |a: &[&str]| a.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            db_arg(args(&[
                "tt-app.exe",
                "--minimized",
                "--db",
                r"C:\t\seed.db"
            ])),
            Some(PathBuf::from(r"C:\t\seed.db"))
        );
        assert_eq!(db_arg(args(&["tt-app.exe", "--minimized"])), None);
        // A flag without a value, or with an empty one, is ignored.
        assert_eq!(db_arg(args(&["tt-app.exe", "--db"])), None);
        assert_eq!(db_arg(args(&["tt-app.exe", "--db", ""])), None);
        // The next flag is not a path.
        assert_eq!(db_arg(args(&["tt-app.exe", "--db", "--minimized"])), None);
    }

    #[test]
    fn resolve_language_saves_choice_from_os_locale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timetracker.db");
        let mut ctx = AppContext::open(path.clone()).unwrap();
        assert_eq!(ctx.resolve_language(Some("ru-RU")).unwrap(), "ru");
        drop(ctx);

        let mut ctx = AppContext::open(path).unwrap();
        assert_eq!(ctx.settings.language, "ru");
        // A saved choice wins over the OS locale.
        assert_eq!(ctx.resolve_language(Some("es-MX")).unwrap(), "ru");
    }
}
