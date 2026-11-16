//! Opens a web page in the default browser.

/// Opens `url` (http/https) with the shell "open" verb.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn open_url(url: &str) -> std::io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    let (verb, url) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; null owner,
    // parameters and directory are allowed.
    let res = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if res as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "ShellExecuteW returned {}",
            res as usize
        )))
    }
}

/// Updates are off outside Windows, so this is never called there.
#[cfg(not(windows))]
pub fn open_url(_url: &str) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}
