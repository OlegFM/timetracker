//! Is there somewhere to show the tray icon? On Windows the taskbar always is. On Linux the
//! icon is a StatusNotifierItem and shows only when a StatusNotifierWatcher (a tray host,
//! e.g. the GNOME AppIndicator extension) owns its D-Bus name; `tray-icon` creates the
//! item without telling us either way.

use std::time::{Duration, Instant};

/// How long a login start waits for the tray host: it may start after us.
pub const TRAY_WAIT_AT_LOGIN: Duration = Duration::from_secs(10);

/// Whether a tray can show its icon, waiting up to `wait` for a host to appear.
#[cfg(windows)]
pub fn available(_wait: Duration) -> bool {
    true
}

/// Whether a tray can show its icon, waiting up to `wait` for a host to appear.
#[cfg(target_os = "linux")]
pub fn available(wait: Duration) -> bool {
    wait_until(watcher_present, wait, Duration::from_millis(500))
}

/// Calls `probe` at once, then every `step` until it returns `true` or `timeout` is over.
/// With a zero `timeout` it is called once.
pub fn wait_until(mut probe: impl FnMut() -> bool, timeout: Duration, step: Duration) -> bool {
    let start = Instant::now();
    loop {
        if probe() {
            return true;
        }
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            return false;
        }
        std::thread::sleep(step.min(left));
    }
}

#[cfg(target_os = "linux")]
fn watcher_present() -> bool {
    let check = || -> zbus::Result<bool> {
        let conn = zbus::blocking::Connection::session()?;
        let name = zbus::names::BusName::try_from("org.kde.StatusNotifierWatcher")?;
        Ok(zbus::blocking::fdo::DBusProxy::new(&conn)?.name_has_owner(name)?)
    };
    match check() {
        Ok(present) => present,
        Err(e) => {
            log::warn!("tray host not detected: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_until_succeeds_once_the_probe_does() {
        let mut calls = 0;
        assert!(wait_until(
            || {
                calls += 1;
                calls == 3
            },
            Duration::from_millis(200),
            Duration::from_millis(1)
        ));
        assert_eq!(calls, 3);
    }

    #[test]
    fn wait_until_gives_up_after_the_timeout() {
        let start = Instant::now();
        assert!(!wait_until(
            || false,
            Duration::from_millis(30),
            Duration::from_millis(5)
        ));
        assert!(start.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn wait_until_zero_probes_once() {
        let mut calls = 0;
        assert!(!wait_until(
            || {
                calls += 1;
                false
            },
            Duration::ZERO,
            Duration::from_millis(5)
        ));
        assert_eq!(calls, 1);
    }
}
