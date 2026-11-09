//! Pure domain logic: entities and local-day boundaries. No I/O.

pub mod billing;
pub mod days;
pub mod format;
pub mod ids;
pub mod model;
pub mod rounding;
pub mod session;

pub use billing::{cost_cents, effective_rate};
pub use chrono_tz::Tz;
pub use days::{local_date, next_local_midnight, split_at_local_midnights, start_of_local_day};
pub use format::{
    CurrencyFormat, DateFormat, FormatPrefs, IntervalFormat, NumberFormat, ParseKeyError,
    TimeFormat, currency_symbol, format_clock, format_date, format_duration, format_money,
    format_number, format_time,
};
pub use ids::{Id, Millis, new_id};
pub use model::{Client, Project, SessionState, Tag, Task, TimeEntry, TimerSession};
pub use rounding::{RoundingMode, rounded_duration_s};
pub use session::{EntryTemplate, day_total, session_total};
