//! One running copy per user session on Unix: the running instance listens on a Unix
//! socket; a second launch connects to it (the connection is the signal) and exits.

use std::io::ErrorKind;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use anyhow::Context;

/// Held by the running instance for its lifetime; removes the socket on drop.
pub struct InstanceGuard {
    path: PathBuf,
    listener: Arc<UnixListener>,
    stop: Arc<AtomicBool>,
    waiter: Option<JoinHandle<()>>,
}

/// `Some` for the first instance. In a second one it asks the running instance to show its
/// window and returns `None`; the caller exits.
pub fn acquire() -> anyhow::Result<Option<InstanceGuard>> {
    let path = super::socket_path(
        std::env::var_os("XDG_RUNTIME_DIR").map(Into::into),
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
            listener: Arc::new(listener),
            stop: Arc::default(),
            waiter: None,
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
        let waiter = std::thread::Builder::new()
            .name("instance-activate".into())
            .spawn(move || {
                while let Ok((_conn, _)) = listener.accept() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    notify();
                }
            });
        match waiter {
            Ok(w) => self.waiter = Some(w),
            Err(e) => log::warn!("activation thread not started: {e}"),
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            self.stop.store(true, Ordering::Release);
            // Wakes `accept`; the thread sees `stop` and ends.
            let _ = UnixStream::connect(&self.path);
            let _ = waiter.join();
        }
        let _ = std::fs::remove_file(&self.path);
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
