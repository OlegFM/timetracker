//! Start with Windows: the value `TimeTracker` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (crate `auto-launch`), launching
//! this executable with `--minimized`.

use std::path::Path;

use anyhow::Context;
use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};

const APP_NAME: &str = "TimeTracker";
/// Command-line flag: start in the tray without showing the main window.
pub const MINIMIZED_ARG: &str = "--minimized";

/// Adds (`true`) or removes (`false`) the Run entry.
pub fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    let launch = launcher()?;
    if enabled {
        launch.enable().context("enable autostart")
    } else {
        launch.disable().context("disable autostart")
    }
}

/// Whether the Run entry exists (and is not disabled in Task Manager).
pub fn is_enabled() -> anyhow::Result<bool> {
    launcher()?.is_enabled().context("read autostart")
}

/// Makes the registry match the setting; touches it only when they differ.
pub fn sync(enabled: bool) -> anyhow::Result<()> {
    if is_enabled()? != enabled {
        set_autostart(enabled)?;
    }
    Ok(())
}

/// Whether the process was started with [`MINIMIZED_ARG`].
pub fn start_minimized(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|a| a == MINIMIZED_ARG)
}

fn launcher() -> anyhow::Result<AutoLaunch> {
    let exe = std::env::current_exe().context("find executable")?;
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&quoted(&exe))
        .set_args(&[MINIMIZED_ARG])
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .build()
        .context("autostart entry")
}

/// The path in quotes: the crate writes `"<path> <args>"` as is, and the path may contain
/// spaces.
fn quoted(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_quoted() {
        assert_eq!(
            quoted(Path::new(r"C:\Program Files\TimeTracker\tt-app.exe")),
            r#""C:\Program Files\TimeTracker\tt-app.exe""#
        );
    }

    #[test]
    fn minimized_flag_is_found_among_args() {
        let args = |a: &[&str]| {
            a.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert!(start_minimized(args(&["tt-app.exe", "--minimized"])));
        assert!(!start_minimized(args(&["tt-app.exe"])));
        assert!(!start_minimized(args(&["tt-app.exe", "--minimized=no"])));
    }
}
