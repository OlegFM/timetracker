//! One running copy per user session on Unix: the running instance listens on a Unix
//! socket; a second launch connects to it (the connection is the signal) and exits.

use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;

/// Held by the running instance for its lifetime; on drop removes the socket file if it is
/// still the one this instance bound. Drop never waits for the listener thread.
pub struct InstanceGuard {
    path: PathBuf,
    /// `(device, inode)` of the socket file right after `bind`.
    id: Option<(u64, u64)>,
    listener: Arc<UnixListener>,
    stop: Arc<AtomicBool>,
}

/// `(device, inode)` of the file at `path`, if it exists.
fn file_id(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// Per-user directory for the socket: `XDG_RUNTIME_DIR` on Linux.
#[cfg(target_os = "linux")]
fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(Into::into)
}

/// Per-user directory for the socket: the app data folder on macOS. There is no
/// `XDG_RUNTIME_DIR`, and `TMPDIR` differs between launchd and Finder launches.
#[cfg(target_os = "macos")]
fn runtime_dir() -> Option<PathBuf> {
    let dir = directories::BaseDirs::new()?.data_dir().join("TimeTracker");
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            log::warn!("create {}: {e}; socket goes to the temp dir", dir.display());
            None
        }
    }
}

/// `Some` for the first instance. In a second one it asks the running instance to show its
/// window and returns `None`; the caller exits.
pub fn acquire() -> anyhow::Result<Option<InstanceGuard>> {
    let path = super::socket_path(
        runtime_dir(),
        std::env::temp_dir(),
        std::env::var("USER").ok(),
    );
    acquire_at(&path)
}

/// [`acquire`] with an explicit socket path.
fn acquire_at(path: &Path) -> anyhow::Result<Option<InstanceGuard>> {
    match UnixListener::bind(path) {
        Ok(listener) => return Ok(Some(InstanceGuard::new(path, listener))),
        Err(e) if e.kind() == ErrorKind::AddrInUse => {}
        Err(e) => return Err(e).context("bind instance socket"),
    }
    match UnixStream::connect(path) {
        // Someone listens: this is a second instance, the connection was the signal.
        Ok(_) => Ok(None),
        // The file is left over from a crashed instance: replace it, once.
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
            std::fs::remove_file(path).context("remove stale instance socket")?;
            let listener = UnixListener::bind(path).context("rebind instance socket")?;
            Ok(Some(InstanceGuard::new(path, listener)))
        }
        Err(e) => Err(e).context("connect to running instance"),
    }
}

impl InstanceGuard {
    fn new(path: &Path, listener: UnixListener) -> Self {
        Self {
            path: path.to_owned(),
            id: file_id(path),
            listener: Arc::new(listener),
            stop: Arc::default(),
        }
    }

    /// Runs `f` on the UI thread (`slint::invoke_from_event_loop`) whenever another launch
    /// signals this instance. Call once.
    pub fn on_activate(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let f = Arc::new(f);
        self.listen(move || {
            let f = f.clone();
            if let Err(e) = slint::invoke_from_event_loop(move || f()) {
                log::warn!("activation dropped: {e}");
            }
        });
    }

    /// Calls `notify` on a background thread for every incoming connection.
    fn listen(&mut self, notify: impl Fn() + Send + 'static) {
        let (listener, stop) = (self.listener.clone(), self.stop.clone());
        let spawned = std::thread::Builder::new()
            .name("instance-activate".into())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok(_) if stop.load(Ordering::Acquire) => break,
                        Ok(_) => notify(),
                        Err(e) => {
                            if !stop.load(Ordering::Acquire) {
                                log::warn!("activation listener stopped: {:?}", e.kind());
                            }
                            break;
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("activation thread not started: {e}");
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Only while the file is still ours: wake `accept` so the thread ends (best effort,
        // never joined: the process is exiting) and remove the file. A file replaced by
        // another instance's socket is left alone.
        if self.id.is_some() && file_id(&self.path) == self.id {
            let _ = UnixStream::connect(&self.path);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_signals_the_first_and_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tt.sock");
        let mut first = acquire_at(&path).unwrap().expect("first instance");
        let (tx, rx) = std::sync::mpsc::channel();
        first.listen(move || tx.send(()).unwrap());
        assert!(acquire_at(&path).unwrap().is_none(), "second instance");
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("activation");
        drop(first);
        assert!(!path.exists(), "socket removed on exit");
        assert!(acquire_at(&path).unwrap().is_some(), "free after exit");
    }

    #[test]
    fn stale_socket_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tt.sock");
        // The file stays, nobody listens.
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(acquire_at(&path).unwrap().is_some());
    }
}
