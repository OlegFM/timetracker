//! Error box for startup failures in release builds (no console there).

/// Shows `text` in a modal error box titled "TimeTracker" and waits for OK.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn show_error(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    let (text, title) = (wide(text), wide("TimeTracker"));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; a null owner
    // window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// Shows `text` in a modal error box titled "TimeTracker" and waits for OK.
#[cfg(not(windows))]
pub fn show_error(text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("TimeTracker")
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
