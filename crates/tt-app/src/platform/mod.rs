//! Platform-specific code (Windows, Linux, macOS). Nothing outside this module uses platform APIs.

/// True when built for macOS (UI wording follows the menu bar vs. tray).
pub const IS_MACOS: bool = cfg!(target_os = "macos");

pub mod autostart;
pub mod dock;
pub mod fatal;
pub mod single_instance;
pub mod startup_notify;
pub mod tray;
pub mod tray_host;
pub mod window_ext;
