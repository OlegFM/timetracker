//! Pure domain logic: entities and local-day boundaries. No I/O.

pub mod days;
pub mod ids;
pub mod model;

pub use chrono_tz::Tz;
pub use days::{local_date, next_local_midnight, split_at_local_midnights, start_of_local_day};
pub use ids::{Id, Millis, new_id};
pub use model::{Client, Project, SessionState, Tag, Task, TimeEntry, TimerSession};
