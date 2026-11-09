//! Application services over `tt-storage`: timer sessions and settings.

pub mod clock;
pub mod error;
pub mod settings;
pub mod timer;

#[cfg(any(test, feature = "test-util"))]
pub use clock::FixedClock;
pub use clock::{Clock, SystemClock};
pub use error::{Result, ServiceError};
pub use settings::{Settings, SettingsService, Theme};
pub use timer::{TimerService, TimerStatus};
