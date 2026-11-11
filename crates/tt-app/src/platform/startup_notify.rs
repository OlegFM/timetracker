//! Completes the tray host's startup notification (Linux, X11).
//!
//! Newer GNOME hands the tray item an XDG activation token right before each tray event;
//! that starts a startup-notification sequence (busy cursor) which lasts until the client
//! sends a `remove` message with that id, or until mutter's ~15 s timeout. The app always
//! runs through X11/XWayland, so the token is completed the X11 way: a `remove: ID=...`
//! message on the root window (freedesktop startup-notification spec; the same code as
//! winit's `remove_activation_token`).
//!
//! The message encoding is plain byte work, so it is compiled (and tested) on every platform.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The startup-notification message `remove: ID="<id>"` with its terminating NUL, split
/// into the 20-byte chunks of X11 client messages (the last one zero-padded).
pub fn remove_message(id: &str) -> Vec<[u8; 20]> {
    let mut text = b"remove: ID=\"".to_vec();
    for &b in id.as_bytes() {
        if b == b'"' || b == b'\\' {
            text.push(b'\\');
        }
        text.push(b);
    }
    text.extend_from_slice(b"\"\0");
    text.chunks(20)
        .map(|chunk| {
            let mut out = [0u8; 20];
            out[..chunk.len()].copy_from_slice(chunk);
            out
        })
        .collect()
}

/// Reads the activation token of the current tray event and completes it. Call it inside
/// the tray's event handlers (the token is only visible there). Per ksni's docs the token
/// may arrive slightly after the event, or never: retried for up to ~20 ms.
#[cfg(target_os = "linux")]
pub fn complete_current_token() {
    for _ in 0..20 {
        if let Some(token) = ksni::current_xdg_activation_token() {
            complete(&token);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(not(target_os = "linux"))]
pub fn complete_current_token() {}

/// Sends `remove: ID=<id>` to the X server's root window, which ends the startup sequence
/// `id` (and its busy cursor). Errors are logged.
#[cfg(target_os = "linux")]
pub fn complete(id: &str) {
    if let Err(e) = send_remove(id) {
        log::warn!("startup notification not completed: {e:#}");
    }
}

#[cfg(target_os = "linux")]
fn send_remove(id: &str) -> anyhow::Result<()> {
    use anyhow::Context;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, WindowClass,
    };

    let (conn, screen) = x11rb::connect(None).context("X11 connection")?;
    let root = conn.setup().roots.get(screen).context("X11 screen")?.root;
    let begin = conn.intern_atom(false, b"_NET_STARTUP_INFO_BEGIN")?;
    let more = conn.intern_atom(false, b"_NET_STARTUP_INFO")?;
    let (begin, more) = (begin.reply()?.atom, more.reply()?.atom);

    // The messages' `window` field: a small unmapped InputOnly window of our own.
    let window = conn.generate_id()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        -100,
        -100,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )?;
    for (i, chunk) in remove_message(id).into_iter().enumerate() {
        let kind = if i == 0 { begin } else { more };
        let event = ClientMessageEvent::new(8, window, kind, chunk);
        conn.send_event(false, root, EventMask::PROPERTY_CHANGE, event)?;
    }
    conn.destroy_window(window)?;
    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(id: &str) -> Vec<u8> {
        remove_message(id).concat()
    }

    #[test]
    fn remove_message_quotes_and_terminates() {
        let id = "gnome-shell/TimeTracker/7744-9-host_TIME1565989";
        let bytes = joined(id);
        let text = format!("remove: ID=\"{id}\"\0");
        assert_eq!(bytes.len() % 20, 0);
        assert!(bytes.len() >= text.len() && bytes.len() < text.len() + 20);
        assert_eq!(&bytes[..text.len()], text.as_bytes());
        assert!(bytes[text.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn remove_message_escapes_quotes_and_backslashes() {
        let bytes = joined("a\"b\\c");
        let text = b"remove: ID=\"a\\\"b\\\\c\"\0";
        assert_eq!(&bytes[..text.len()], text);
        assert!(bytes[text.len()..].iter().all(|&b| b == 0));
    }

    #[test]
    fn remove_message_exact_multiple_of_20_still_ends_with_nul() {
        // `remove: ID=""` is 13 bytes, so a 27-byte id makes 40 bytes before the NUL.
        let id = "x".repeat(27);
        let chunks = remove_message(&id);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[2][0], 0);
        let text = format!("remove: ID=\"{id}\"");
        assert_eq!(text.len(), 40);
        assert_eq!(&chunks[..2].concat()[..], text.as_bytes());
    }
}
