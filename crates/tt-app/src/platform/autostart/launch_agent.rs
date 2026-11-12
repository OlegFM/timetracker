//! macOS storage of the autostart entry: `~/Library/LaunchAgents/TimeTracker.plist`,
//! written by the crate `auto-launch`. The "Allow in the Background" switch of System
//! Settings is not stored there, so it is not read.
//!
//! The parsing is plain text work, so it is compiled (and tested) on every platform.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Component, Path};

use super::StoredEntry;

/// Reads `ProgramArguments` of a LaunchAgent plist as the command `sync` compares:
/// `"<arg0>" <arg1> <arg2>...`. The crate writes the strings without XML escaping, so they
/// are taken as they are. `disabled` is always `false`.
pub fn parse_launch_agent(text: &str) -> StoredEntry {
    let Some((_, rest)) = text.split_once("<key>ProgramArguments</key>") else {
        return StoredEntry::default();
    };
    let Some((_, rest)) = rest.split_once("<array>") else {
        return StoredEntry::default();
    };
    let array = rest.split_once("</array>").map_or(rest, |(a, _)| a);
    let mut args = array
        .split("<string>")
        .skip(1)
        .filter_map(|s| s.split_once("</string>").map(|(v, _)| v));
    let Some(exe) = args.next().filter(|e| !e.is_empty()) else {
        return StoredEntry::default();
    };
    let mut command = format!("\"{exe}\"");
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    StoredEntry {
        command: Some(command),
        disabled: false,
    }
}

/// Whether the executable stays where it is: macOS runs a program opened from a `.dmg`
/// (`/Volumes/...`) or from Downloads (App Translocation) from a temporary copy, and an
/// entry pointing there would break.
pub fn permanent_location(exe: &Path) -> bool {
    !exe.starts_with("/Volumes")
        && !exe
            .components()
            .any(|c| c == Component::Normal("AppTranslocation".as_ref()))
}

/// The stored entry, or the default (no command, not disabled) when there is no file.
#[cfg(target_os = "macos")]
pub fn stored_entry() -> StoredEntry {
    // Built from the home directory like `auto-launch` does, so that this is the very file
    // the crate writes.
    directories::BaseDirs::new()
        .map(|d| d.home_dir().join("Library/LaunchAgents/TimeTracker.plist"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_launch_agent(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::{SyncPlan, plan_sync};
    use super::*;

    const WRITTEN: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n  <dict>\n  <key>Label</key>\n  <string>TimeTracker</string>\n  <key>AssociatedBundleIdentifiers</key>\n  <array><string>io.github.olegfm.timetracker</string></array>\n  <key>ProgramArguments</key>\n  <array><string>/Applications/Time Tracker.app/Contents/MacOS/TimeTracker</string><string>--minimized</string></array>\n  <key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>";

    #[test]
    fn launch_agent_round_trip_keeps_the_entry() {
        let e = parse_launch_agent(WRITTEN);
        let expected = "\"/Applications/Time Tracker.app/Contents/MacOS/TimeTracker\" --minimized";
        assert_eq!(
            e,
            StoredEntry {
                command: Some(expected.into()),
                disabled: false
            }
        );
        assert_eq!(
            plan_sync(true, e.command.as_deref(), e.disabled, expected),
            SyncPlan::Keep
        );
    }

    #[test]
    fn launch_agent_round_trip_moved_app_is_registered_again() {
        let e = parse_launch_agent(WRITTEN);
        let moved =
            "\"/Users/ann/Applications/TimeTracker.app/Contents/MacOS/TimeTracker\" --minimized";
        assert_eq!(
            plan_sync(true, e.command.as_deref(), false, moved),
            SyncPlan::Register
        );
    }

    #[test]
    fn launch_agent_without_program_arguments_is_empty() {
        assert_eq!(parse_launch_agent(""), StoredEntry::default());
        assert_eq!(
            parse_launch_agent(
                "<plist><dict><key>Label</key><string>TimeTracker</string></dict></plist>"
            ),
            StoredEntry::default()
        );
    }

    #[test]
    fn permanent_location_rejects_dmg_and_translocation() {
        assert!(permanent_location(Path::new(
            "/Applications/TimeTracker.app/Contents/MacOS/TimeTracker"
        )));
        assert!(permanent_location(Path::new(
            "/Users/ann/Applications/TimeTracker.app/Contents/MacOS/TimeTracker"
        )));
        assert!(!permanent_location(Path::new(
            "/Volumes/TimeTracker/TimeTracker.app/Contents/MacOS/TimeTracker"
        )));
        assert!(!permanent_location(Path::new(
            "/private/var/folders/ab/cd/T/AppTranslocation/0F1E/d/TimeTracker.app/Contents/MacOS/TimeTracker"
        )));
    }
}
