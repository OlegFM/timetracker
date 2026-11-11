//! Windows storage of the autostart entry: the `Run` value and Task Manager's
//! `StartupApproved\Run` switch.

use windows_registry::CURRENT_USER;

use super::{APP_NAME, StoredEntry};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Whether a `StartupApproved\Run` value says "disabled": Windows writes 0x02 / 0x06 when
/// the entry is enabled and 0x03 (plus a timestamp) when Task Manager disabled it.
pub fn startup_approved_disabled(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// The Run command and Task Manager's switch, read from the registry.
pub fn stored_entry() -> StoredEntry {
    let command = CURRENT_USER
        .open(RUN_KEY)
        .and_then(|k| k.get_string(APP_NAME))
        .ok();
    let disabled = CURRENT_USER
        .open(APPROVED_KEY)
        .and_then(|k| k.get_value(APP_NAME))
        .is_ok_and(|v| startup_approved_disabled(&v));
    StoredEntry { command, disabled }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
