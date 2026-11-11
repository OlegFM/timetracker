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

/// Where the instance socket lives: `timetracker.sock` in `XDG_RUNTIME_DIR` (per-user,
/// mode 0700) when it is set and not empty, else a per-user file in the temp directory.
#[cfg_attr(windows, allow(dead_code))]
pub fn socket_path(
    runtime_dir: Option<PathBuf>,
    temp_dir: PathBuf,
    user: Option<String>,
) -> PathBuf {
    match runtime_dir.filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir.join("timetracker.sock"),
        None => temp_dir.join(format!(
            "timetracker-{}.sock",
            user.as_deref().unwrap_or("user")
        )),
    }
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
}
