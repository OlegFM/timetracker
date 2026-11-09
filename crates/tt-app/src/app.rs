//! Application context owned by the UI thread, and app-wide UI settings helpers.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Context;
use directories::BaseDirs;
use slint::ComponentHandle;
use tt_services::{Settings, SettingsService, SystemClock, TimerService};
use tt_storage::Db;

use crate::vm::lang;
use crate::{MainWindow, Theme};

/// Folder name under `%APPDATA%` (data) and `%LOCALAPPDATA%` (logs).
const APP_DIR: &str = "TimeTracker";

pub struct AppContext {
    pub db: Db,
    pub timer: TimerService<SystemClock>,
    pub settings: Settings,
    pub db_path: PathBuf,
}

/// Shared handle used by controllers (UI thread only).
pub type Ctx = Rc<RefCell<AppContext>>;

impl AppContext {
    /// Opens (creating if needed) `%APPDATA%\TimeTracker\timetracker.db`.
    pub fn open_default() -> anyhow::Result<AppContext> {
        let dirs = BaseDirs::new().context("no user profile folders")?;
        Self::open(dirs.data_dir().join(APP_DIR).join("timetracker.db"))
    }

    /// Opens the database at `db_path` (creating its folder) and loads the settings.
    pub fn open(db_path: PathBuf) -> anyhow::Result<AppContext> {
        if let Some(dir) = db_path.parent() {
            std::fs::create_dir_all(dir).context("create data folder")?;
        }
        let db = Db::open(&db_path).context("open database")?;
        let settings = SettingsService::load(&db).context("load settings")?;
        Ok(AppContext {
            db,
            timer: TimerService::new(SystemClock),
            settings,
            db_path,
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

/// `%LOCALAPPDATA%\TimeTracker\logs`.
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

pub fn apply_theme(ui: &MainWindow, theme: tt_services::Theme) {
    ui.global::<Theme>()
        .set_dark(theme == tt_services::Theme::Dark);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_folder_and_loads_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("timetracker.db");
        let ctx = AppContext::open(path.clone()).unwrap();
        assert!(path.exists());
        assert_eq!(ctx.db_path, path);
        assert_eq!(ctx.settings, Settings::default());
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
