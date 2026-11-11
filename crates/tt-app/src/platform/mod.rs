//! Platform-specific code (Windows, Linux, macOS). Nothing outside this module uses platform APIs.

pub mod autostart;
pub mod dock;
pub mod fatal;
pub mod single_instance;
pub mod tray;
pub mod tray_host;
pub mod window_ext;
