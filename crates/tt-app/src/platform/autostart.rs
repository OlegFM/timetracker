//! Start with Windows: the value `TimeTracker` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (crate `auto-launch`), launching
//! this executable with `--minimized`.
//!
//! Task Manager's "Startup apps" switch is stored apart, as a value of the same name under
//! `...\Explorer\StartupApproved\Run`. That switch is the user's choice: [`sync`] never
//! turns it back on.

use std::path::Path;

use anyhow::Context;
use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};
use windows_registry::CURRENT_USER;

const APP_NAME: &str = "TimeTracker";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
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

/// What [`sync`] does about the Run entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlan {
    /// Registry and setting agree.
    Keep,
    /// Write the entry (missing, or it launches another executable).
    Register,
    /// Remove the entry (only when it starts this executable).
    Unregister,
    /// The setting is on but Task Manager disabled the entry: the setting becomes off.
    DisabledByUser,
}

/// Whether a `StartupApproved\Run` value says "disabled": Windows writes 0x02 / 0x06 when
/// the entry is enabled and 0x03 (plus a timestamp) when Task Manager disabled it.
pub fn startup_approved_disabled(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// Decides what to do for the autostart `want`ed by the settings, given the stored Run
/// command (`run`), whether Task Manager disabled it and the command that should be there.
pub fn plan_sync(want: bool, run: Option<&str>, disabled: bool, expected: &str) -> SyncPlan {
    match (want, run) {
        (true, None) => SyncPlan::Register,
        (true, Some(_)) if disabled => SyncPlan::DisabledByUser,
        (true, Some(cmd)) if !cmd.trim().eq_ignore_ascii_case(expected) => SyncPlan::Register,
        (false, Some(cmd)) if runs_same_exe(cmd, expected) => SyncPlan::Unregister,
        // Off: an entry of another executable under our name is not ours to remove.
        (true, Some(_)) | (false, _) => SyncPlan::Keep,
    }
}

/// The executable a Run command starts: the quoted first part, else up to the first space.
fn command_exe(cmd: &str) -> &str {
    let cmd = cmd.trim_start();
    match cmd.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => cmd.split_whitespace().next().unwrap_or(""),
    }
}

/// Whether two Run commands start the same executable (paths compared ignoring case;
/// arguments ignored).
pub fn runs_same_exe(cmd: &str, other: &str) -> bool {
    let exe = command_exe(cmd);
    !exe.is_empty() && exe.eq_ignore_ascii_case(command_exe(other))
}

/// Makes the registry match the setting and returns the setting to keep: `false` when
/// the user disabled the entry in Task Manager (it is then left as it is).
pub fn sync(want: bool) -> anyhow::Result<bool> {
    let run = CURRENT_USER
        .open(RUN_KEY)
        .and_then(|k| k.get_string(APP_NAME))
        .ok();
    let disabled = CURRENT_USER
        .open(APPROVED_KEY)
        .and_then(|k| k.get_value(APP_NAME))
        .is_ok_and(|v| startup_approved_disabled(&v));
    let expected = format!("{} {MINIMIZED_ARG}", quoted(&std::env::current_exe()?));
    match plan_sync(want, run.as_deref(), disabled, &expected) {
        SyncPlan::Keep => Ok(want),
        SyncPlan::Register => set_autostart(true).map(|()| want),
        SyncPlan::Unregister => set_autostart(false).map(|()| want),
        SyncPlan::DisabledByUser => Ok(false),
    }
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

    const EXPECTED: &str = r#""C:\Apps\tt-app.exe" --minimized"#;

    #[test]
    fn startup_approved_values() {
        let enabled = [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let enabled_alt = [0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let disabled = [
            0x03, 0, 0, 0, 0xd0, 0x5e, 0x9a, 0x11, 0x25, 0x8c, 0xdb, 0x01,
        ];
        assert!(!startup_approved_disabled(&enabled));
        assert!(!startup_approved_disabled(&enabled_alt));
        assert!(startup_approved_disabled(&disabled));
        assert!(!startup_approved_disabled(&[]));
    }

    #[test]
    fn sync_plan_follows_the_setting_and_the_stored_command() {
        let same = Some(EXPECTED);
        // Wanted and present: nothing to do (also for another letter case of the path).
        assert_eq!(plan_sync(true, same, false, EXPECTED), SyncPlan::Keep);
        assert_eq!(
            plan_sync(true, Some(&EXPECTED.to_uppercase()), false, EXPECTED),
            SyncPlan::Keep
        );
        // Missing, or pointing at another executable (the app was moved): write it again.
        assert_eq!(plan_sync(true, None, false, EXPECTED), SyncPlan::Register);
        let moved = Some(r#""D:\Old\tt-app.exe" --minimized"#);
        assert_eq!(plan_sync(true, moved, false, EXPECTED), SyncPlan::Register);
        // Not wanted: remove a leftover entry.
        assert_eq!(
            plan_sync(false, same, false, EXPECTED),
            SyncPlan::Unregister
        );
        assert_eq!(plan_sync(false, None, false, EXPECTED), SyncPlan::Keep);
    }

    #[test]
    fn the_exe_of_a_command_is_compared_without_args_and_case() {
        assert!(runs_same_exe(EXPECTED, EXPECTED));
        assert!(runs_same_exe(&EXPECTED.to_lowercase(), EXPECTED));
        // Without the flag, or unquoted: still our executable.
        assert!(runs_same_exe(
            r#" "C:\Apps\tt-app.exe""#,
            r#""C:\Apps\tt-app.exe" --minimized"#
        ));
        assert!(runs_same_exe(
            r"C:\Apps\tt-app.exe --minimized",
            r#""C:\Apps\tt-app.exe" --minimized"#
        ));
        assert!(!runs_same_exe(
            r#""D:\Other\tt-app.exe" --minimized"#,
            r#""C:\Apps\tt-app.exe" --minimized"#
        ));
        assert!(!runs_same_exe("", EXPECTED));
    }

    #[test]
    fn setting_off_removes_only_our_own_entry() {
        assert_eq!(
            plan_sync(false, Some(EXPECTED), false, EXPECTED),
            SyncPlan::Unregister
        );
        // Another copy of the app (or another program under the same name) keeps its entry.
        let other = Some(r#""D:\Other\tt-app.exe" --minimized"#);
        assert_eq!(plan_sync(false, other, false, EXPECTED), SyncPlan::Keep);
    }

    #[test]
    fn task_manager_disable_wins_over_the_setting() {
        assert_eq!(
            plan_sync(true, Some(EXPECTED), true, EXPECTED),
            SyncPlan::DisabledByUser
        );
        // Even when the stored command is outdated: the user's switch is respected.
        assert_eq!(
            plan_sync(true, Some("old"), true, EXPECTED),
            SyncPlan::DisabledByUser
        );
        // A setting that is already off is not affected.
        assert_eq!(plan_sync(false, None, true, EXPECTED), SyncPlan::Keep);
    }
}
