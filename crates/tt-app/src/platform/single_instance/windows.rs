//! One running copy per user session: a named mutex marks the running instance, a named
//! auto-reset event lets a second launch ask it to show its window.
//!
//! The only Win32 FFI of the app besides [`crate::platform::fatal`]; every call has a SAFETY note.
#![allow(unsafe_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use anyhow::bail;
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
    WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

const MUTEX_NAME: &str = "Local\\TimeTracker.Instance";
const EVENT_NAME: &str = "Local\\TimeTracker.Activate";

/// An owned kernel handle, closed on drop.
struct Handle(HANDLE);

// SAFETY: a kernel handle is a process-wide value; the event is only waited on and set,
// which Win32 allows from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid handle owned by this value and closed only here.
        unsafe { CloseHandle(self.0) };
    }
}

/// Held by the running instance for its lifetime.
pub struct InstanceGuard {
    _mutex: Handle,
    event: Arc<Handle>,
    stop: Arc<AtomicBool>,
    waiter: Option<JoinHandle<()>>,
}

/// `Some` for the first instance. In a second one it asks the running instance to show its
/// window and returns `None`; the caller exits.
pub fn acquire() -> anyhow::Result<Option<InstanceGuard>> {
    acquire_named(MUTEX_NAME, EVENT_NAME)
}

fn acquire_named(mutex_name: &str, event_name: &str) -> anyhow::Result<Option<InstanceGuard>> {
    let name = wide(mutex_name);
    // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call; null
    // attributes = default security, not inherited.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    // SAFETY: no other Win32 call since `CreateMutexW` on this thread.
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if mutex.is_null() {
        bail!("create instance mutex");
    }
    let mutex = Handle(mutex);
    if already {
        signal_existing(event_name);
        return Ok(None);
    }
    let name = wide(event_name);
    // SAFETY: as above; auto-reset (`bManualReset` = 0), initially not signaled.
    let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        bail!("create activation event");
    }
    Ok(Some(InstanceGuard {
        _mutex: mutex,
        event: Arc::new(Handle(event)),
        stop: Arc::default(),
        waiter: None,
    }))
}

/// Sets the running instance's activation event (nothing happens if it has none yet).
fn signal_existing(event_name: &str) {
    let name = wide(event_name);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if event.is_null() {
        log::warn!("running instance not found");
        return;
    }
    let event = Handle(event);
    // SAFETY: plain calls with a valid event handle; ASFW_ANY lets the running instance
    // bring its window to the front (this process has the user's input).
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
        SetEvent(event.0);
    }
}

impl InstanceGuard {
    /// Runs `f` on the UI thread (`slint::invoke_from_event_loop`) whenever another launch
    /// signals this instance. Call once.
    pub fn on_activate(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let (event, stop, f) = (self.event.clone(), self.stop.clone(), Arc::new(f));
        let waiter = std::thread::Builder::new()
            .name("instance-activate".into())
            .spawn(move || {
                loop {
                    // SAFETY: the handle stays open while this thread holds its `Arc`.
                    unsafe { WaitForSingleObject(event.0, INFINITE) };
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let f = f.clone();
                    if let Err(e) = slint::invoke_from_event_loop(move || f()) {
                        log::warn!("activation dropped: {e}");
                    }
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
        let Some(waiter) = self.waiter.take() else {
            return;
        };
        self.stop.store(true, Ordering::Release);
        // SAFETY: valid event handle (owned by `self.event`); wakes the waiting thread.
        unsafe { SetEvent(self.event.0) };
        let _ = waiter.join();
    }
}

/// NUL-terminated UTF-16.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;

    use super::*;

    #[test]
    fn second_acquire_signals_the_first_and_fails() {
        let id = std::process::id();
        let (m, e) = (
            format!("Local\\TimeTracker.Test{id}.Instance"),
            format!("Local\\TimeTracker.Test{id}.Activate"),
        );
        let first = acquire_named(&m, &e).unwrap().expect("first instance");
        // SAFETY: valid handle; zero timeout = just test the state.
        let signaled = || unsafe { WaitForSingleObject(first.event.0, 0) } == WAIT_OBJECT_0;
        assert!(!signaled());

        assert!(acquire_named(&m, &e).unwrap().is_none(), "second instance");
        assert!(signaled(), "the running instance was asked to activate");
        assert!(!signaled(), "auto-reset: one activation per launch");

        drop(first);
        assert!(acquire_named(&m, &e).unwrap().is_some(), "free after exit");
    }
}
