//! File log (`%LOCALAPPDATA%\TimeTracker\logs\tt.log`, level Warn) and panic hook.
//!
//! Log only ids and error codes: never descriptions, names or other user text.

use std::fs::OpenOptions;
use std::path::{Component, Path};

use simplelog::{Config, LevelFilter, WriteLogger};

use crate::app;

/// Best effort: without a writable log folder the app runs without a log.
pub fn init() {
    if let Some(dir) = app::log_dir()
        && std::fs::create_dir_all(&dir).is_ok()
        && let Ok(file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("tt.log"))
    {
        let _ = WriteLogger::init(LevelFilter::Warn, Config::default(), file);
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The location only: panic messages may carry user data.
        match info.location() {
            Some(l) => log::error!("panic at {}:{}", short_path(l.file()), l.line()),
            None => log::error!("panic"),
        }
        default_hook(info);
    }));
}

/// Source path without the user-specific prefix (e.g. the cargo registry under the
/// user profile): absolute paths keep their last three components.
fn short_path(file: &str) -> String {
    let path = Path::new(file);
    if !path.is_absolute() {
        return file.to_owned();
    }
    let tail: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect();
    tail[tail.len().saturating_sub(3)..].join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_path_hides_user_profile() {
        // Separators and absolute-path syntax differ per platform.
        #[cfg(windows)]
        let (abs, rel) = (
            r"C:\Users\someone\.cargo\registry\src\idx\slint-1.18.1\src\lib.rs",
            r"crates\tt-app\src\app.rs",
        );
        #[cfg(not(windows))]
        let (abs, rel) = (
            "/home/someone/.cargo/registry/src/idx/slint-1.18.1/src/lib.rs",
            "crates/tt-app/src/app.rs",
        );
        assert_eq!(short_path(abs), "slint-1.18.1/src/lib.rs");
        assert_eq!(short_path(rel), rel);
    }
}
