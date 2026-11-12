//! One running copy per user session. A second launch asks the running instance to show
//! its window and exits.
//!
//! Windows: a named mutex plus an event. Unix: a socket in the runtime directory.

use std::path::PathBuf;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{InstanceGuard, acquire};
#[cfg(windows)]
pub use windows::{InstanceGuard, acquire};

/// Longest socket path we bind: `sun_path` is 104 bytes on macOS (108 on Linux) including
/// the terminating NUL; take the smaller.
const SUN_PATH_MAX: usize = 103;

/// Where the instance socket lives: `timetracker.sock` in `runtime_dir` when it is given,
/// not empty and the resulting path fits `sun_path`, else a per-user file in the temp
/// directory. `runtime_dir` is `XDG_RUNTIME_DIR` (per-user, mode 0700) on Linux and the
/// app data folder on macOS, which, unlike `TMPDIR`, is the same for launchd and Finder.
#[cfg_attr(windows, allow(dead_code))]
pub fn socket_path(
    runtime_dir: Option<PathBuf>,
    temp_dir: PathBuf,
    user: Option<String>,
) -> PathBuf {
    runtime_dir
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join("timetracker.sock"))
        .filter(|p| p.as_os_str().len() <= SUN_PATH_MAX)
        .unwrap_or_else(|| {
            temp_dir.join(format!(
                "timetracker-{}.sock",
                user.as_deref().unwrap_or("user")
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_in_runtime_dir_else_per_user_temp() {
        let tmp = PathBuf::from("/tmp");
        assert_eq!(
            socket_path(
                Some("/run/user/1000".into()),
                tmp.clone(),
                Some("ann".into())
            ),
            PathBuf::from("/run/user/1000/timetracker.sock")
        );
        assert_eq!(
            socket_path(Some("".into()), tmp.clone(), Some("ann".into())),
            PathBuf::from("/tmp/timetracker-ann.sock")
        );
        assert_eq!(
            socket_path(None, tmp, None),
            PathBuf::from("/tmp/timetracker-user.sock")
        );
    }

    #[test]
    fn socket_path_in_app_data_dir_on_macos_layout() {
        let dir = PathBuf::from("/Users/ann/Library/Application Support/TimeTracker");
        assert_eq!(
            socket_path(
                Some(dir.clone()),
                "/var/folders/x/T".into(),
                Some("ann".into())
            ),
            dir.join("timetracker.sock")
        );
    }

    #[test]
    fn socket_path_too_long_for_sun_path_falls_back_to_temp() {
        let long = PathBuf::from(format!(
            "/Users/{}/Library/Application Support/TimeTracker",
            "a".repeat(60)
        ));
        assert_eq!(
            socket_path(Some(long), "/tmp".into(), Some("ann".into())),
            PathBuf::from("/tmp/timetracker-ann.sock")
        );
    }
}
