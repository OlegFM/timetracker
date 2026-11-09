use serde::{Deserialize, Serialize};

use crate::ids::{Id, Millis};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub id: Id,
    pub name: String,
    pub archived_at: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub name: String,
    /// "#rrggbb"
    pub color: String,
    pub client_id: Option<Id>,
    pub is_billable: bool,
    /// Cents per hour.
    pub billable_rate: Option<i64>,
    /// Seconds.
    pub estimated_time: Option<i64>,
    pub archived_at: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id,
    pub project_id: Id,
    pub name: String,
    /// Seconds.
    pub estimated_time: Option<i64>,
    pub done_at: Option<Millis>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: Id,
    pub name: String,
    pub created_at: Millis,
    pub updated_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeEntry {
    pub id: Id,
    pub session_id: Option<Id>,
    pub description: String,
    /// Half-open interval `[start, end)`; `end == None` means running.
    pub start: Millis,
    pub end: Option<Millis>,
    pub billable: bool,
    pub project_id: Option<Id>,
    pub task_id: Option<Id>,
    pub tag_ids: Vec<Id>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

impl TimeEntry {
    /// Duration in ms; a running entry is measured up to `now`. Never negative.
    pub fn duration_ms(&self, now: Millis) -> i64 {
        (self.end.unwrap_or(now) - self.start).max(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerSession {
    pub id: Id,
    pub started_at: Millis,
    pub ended_at: Option<Millis>,
    pub state: SessionState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::new_id;

    fn entry(start: Millis, end: Option<Millis>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: String::new(),
            start,
            end,
            billable: false,
            project_id: None,
            task_id: None,
            tag_ids: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn duration_closed_running_and_clamped() {
        assert_eq!(entry(1_000, Some(4_000)).duration_ms(99_000), 3_000);
        assert_eq!(entry(1_000, None).duration_ms(6_000), 5_000);
        assert_eq!(entry(5_000, None).duration_ms(1_000), 0);
    }
}
