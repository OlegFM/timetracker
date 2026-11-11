//! Linux (XDG) storage of the autostart entry: `~/.config/autostart/TimeTracker.desktop`,
//! written by the crate `auto-launch`. GNOME's "Startup Applications" switch is the key
//! `X-GNOME-Autostart-enabled` of that same file.
//!
//! The parsing is plain text work, so it is compiled (and tested) on every platform.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::PathBuf;

use super::StoredEntry;

/// Reads the `Exec` command and the "disabled" flags (`Hidden=true`,
/// `X-GNOME-Autostart-enabled=false`) of a `.desktop` file. Lines are scanned in order and
/// the first occurrence of a key wins; groups are not told apart (the crate writes one).
pub fn parse_desktop_entry(text: &str) -> StoredEntry {
    let mut command = None;
    let mut hidden = None;
    let mut gnome_enabled = None;
    for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
        let slot = match key.trim() {
            "Exec" => &mut command,
            "Hidden" => &mut hidden,
            "X-GNOME-Autostart-enabled" => &mut gnome_enabled,
            _ => continue,
        };
        slot.get_or_insert_with(|| value.trim().to_owned());
    }
    let is =
        |v: &Option<String>, want: &str| v.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(want));
    StoredEntry {
        disabled: is(&hidden, "true") || is(&gnome_enabled, "false"),
        command,
    }
}

/// What the entry should start: the AppImage file when run from one (the mounted copy
/// disappears on exit), else the current executable.
pub fn launch_target(appimage: Option<OsString>, current_exe: PathBuf) -> PathBuf {
    match appimage {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => current_exe,
    }
}

/// The stored entry, or the default (no command, not disabled) when there is no file.
#[cfg(target_os = "linux")]
pub fn stored_entry() -> StoredEntry {
    // Built from the home directory like `auto-launch` does, not from `XDG_CONFIG_HOME`,
    // so that this is the very file the crate writes.
    directories::BaseDirs::new()
        .map(|d| d.home_dir().join(".config/autostart/TimeTracker.desktop"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_desktop_entry(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_exec_and_enabled() {
        let e = parse_desktop_entry(
            "[Desktop Entry]\nType=Application\nName=TimeTracker\nExec=\"/opt/tt/timetracker\" --minimized\nTerminal=false",
        );
        assert_eq!(
            e,
            StoredEntry {
                command: Some("\"/opt/tt/timetracker\" --minimized".into()),
                disabled: false
            }
        );
    }

    #[test]
    fn desktop_entry_disabled_by_gnome_or_hidden() {
        for flag in [
            "X-GNOME-Autostart-enabled=false",
            "Hidden=true",
            "Hidden=True",
        ] {
            let e = parse_desktop_entry(&format!(
                "[Desktop Entry]\nExec=/a/tt --minimized\n{flag}\n"
            ));
            assert!(e.disabled, "{flag}");
        }
        let e = parse_desktop_entry(
            "[Desktop Entry]\nExec=/a/tt\nX-GNOME-Autostart-enabled=true\nHidden=false\n",
        );
        assert!(!e.disabled);
    }

    #[test]
    fn desktop_entry_empty() {
        assert_eq!(parse_desktop_entry(""), StoredEntry::default());
    }

    #[test]
    fn launch_target_prefers_the_appimage_file() {
        let mounted = PathBuf::from("/tmp/.mount_TimeTrXyz/usr/bin/timetracker");
        assert_eq!(
            launch_target(
                Some("/home/u/Apps/TimeTracker-0.2.0-x86_64.AppImage".into()),
                mounted.clone()
            ),
            PathBuf::from("/home/u/Apps/TimeTracker-0.2.0-x86_64.AppImage")
        );
        assert_eq!(launch_target(None, mounted.clone()), mounted);
        assert_eq!(launch_target(Some("".into()), mounted.clone()), mounted);
    }
}
