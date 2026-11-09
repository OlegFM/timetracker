//! Timer bar / mini timer view.

use slint::SharedString;
use tt_domain::{
    EntryTemplate, FormatPrefs, Id, Millis, SessionState, format_clock, format_duration, local_date,
};
use tt_services::TimerStatus;

use crate::TimerView;
use crate::vm::pickers::{Catalog, hex_color, option_label};

/// Template of the active session: the stored one, else the current entry's attributes.
pub fn active_template(status: &TimerStatus) -> Option<EntryTemplate> {
    status.session.as_ref()?;
    status
        .template
        .clone()
        .or_else(|| status.current.as_ref().map(EntryTemplate::from_entry))
}

/// View of `status`; without an active session it shows `draft` and a zero clock.
pub fn timer_view(
    status: &TimerStatus,
    draft: &EntryTemplate,
    catalog: &Catalog,
    prefs: &FormatPrefs,
    now: Millis,
) -> TimerView {
    let state = match status.session.as_ref().map(|s| s.state) {
        Some(SessionState::Running) => 1,
        Some(SessionState::Paused) => 2,
        Some(SessionState::Stopped) | None => 0,
    };
    let t = active_template(status).unwrap_or_else(|| draft.clone());
    let project = t.project_id.and_then(|id| catalog.project(id));
    let task = t.task_id.and_then(|id| catalog.task(id));
    let started_earlier = status
        .session
        .as_ref()
        .is_some_and(|s| local_date(s.started_at, prefs.tz) < local_date(now, prefs.tz));
    let tags: Vec<&str> = catalog
        .tags
        .iter()
        .filter(|tag| t.tag_ids.contains(&tag.id))
        .map(|tag| tag.name.as_str())
        .collect();
    TimerView {
        state,
        description: t.description.as_str().into(),
        project_label: project
            .map(|p| option_label(p, task))
            .unwrap_or_default()
            .into(),
        project_color: project.map(|p| hex_color(&p.color)).unwrap_or_default(),
        project_id: id_text(project.map(|p| p.id)),
        task_id: id_text(task.filter(|_| project.is_some()).map(|t| t.id)),
        tags_label: tags.join(", ").into(),
        billable: t.billable,
        clock: format_clock(status.session_ms / 1000).into(),
        today: if started_earlier {
            format_duration(status.today_ms / 1000, prefs).into()
        } else {
            SharedString::new()
        },
    }
}

fn id_text(id: Option<Id>) -> SharedString {
    id.map(|id| id.to_string().into()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Color;
    use tt_domain::{IntervalFormat, TimerSession, Tz, new_id};

    use crate::vm::pickers::tests::{project, tag, task};

    const HOUR: i64 = 3_600_000;
    /// A UTC midnight.
    const DAY0: Millis = 20_000 * 24 * HOUR;

    fn prefs() -> FormatPrefs {
        FormatPrefs {
            interval: IntervalFormat::HoursMinutesSecondsColon,
            tz: Tz::UTC,
            ..FormatPrefs::default()
        }
    }

    fn status(
        state: SessionState,
        started_at: Millis,
        session_ms: i64,
        today_ms: i64,
    ) -> TimerStatus {
        TimerStatus {
            session: Some(TimerSession {
                id: new_id(),
                started_at,
                ended_at: None,
                state,
            }),
            current: None,
            template: Some(EntryTemplate {
                description: "Reports page".into(),
                ..Default::default()
            }),
            session_ms,
            today_ms,
        }
    }

    #[test]
    fn stopped_shows_draft_and_zero_clock() {
        let draft = EntryTemplate {
            description: "Next thing".into(),
            billable: true,
            ..Default::default()
        };
        let v = timer_view(
            &TimerStatus::default(),
            &draft,
            &Catalog::default(),
            &prefs(),
            DAY0 + HOUR,
        );
        assert_eq!(v.state, 0);
        assert_eq!(v.clock, "0:00:00");
        assert_eq!(v.description, "Next thing");
        assert!(v.billable);
        assert_eq!(v.today, "");
        assert_eq!(v.project_label, "");
    }

    #[test]
    fn paused_shows_session_sum_and_template() {
        let s = status(SessionState::Paused, DAY0 + 9 * HOUR, HOUR + 15 * 60_000, 0);
        let draft = EntryTemplate {
            description: "ignored".into(),
            ..Default::default()
        };
        let v = timer_view(&s, &draft, &Catalog::default(), &prefs(), DAY0 + 12 * HOUR);
        assert_eq!(v.state, 2);
        assert_eq!(v.clock, "1:15:00");
        assert_eq!(v.description, "Reports page");
    }

    #[test]
    fn running_state_is_one() {
        let s = status(SessionState::Running, DAY0 + HOUR, 5_000, 5_000);
        let v = timer_view(
            &s,
            &EntryTemplate::default(),
            &Catalog::default(),
            &prefs(),
            DAY0 + HOUR + 5_000,
        );
        assert_eq!(v.state, 1);
        assert_eq!(v.clock, "0:00:05");
    }

    #[test]
    fn today_is_empty_when_session_started_today() {
        let s = status(
            SessionState::Running,
            DAY0 + 30 * 60_000,
            2 * HOUR,
            2 * HOUR,
        );
        let v = timer_view(
            &s,
            &EntryTemplate::default(),
            &Catalog::default(),
            &prefs(),
            DAY0 + 3 * HOUR,
        );
        assert_eq!(v.today, "");
    }

    #[test]
    fn today_shows_todays_part_of_older_session() {
        // Started yesterday 23:00, now 01:15: 2:15 in the session, 1:15 of it today.
        let s = status(
            SessionState::Running,
            DAY0 - HOUR,
            2 * HOUR + 15 * 60_000,
            HOUR + 15 * 60_000,
        );
        let now = DAY0 + HOUR + 15 * 60_000;
        let v = timer_view(
            &s,
            &EntryTemplate::default(),
            &Catalog::default(),
            &prefs(),
            now,
        );
        assert_eq!(v.clock, "2:15:00");
        assert_eq!(v.today, "1:15:00");

        let hm = FormatPrefs {
            interval: IntervalFormat::HoursMinutes,
            hour_suffix: "ч".into(),
            minute_suffix: "мин".into(),
            ..prefs()
        };
        let v = timer_view(&s, &EntryTemplate::default(), &Catalog::default(), &hm, now);
        assert_eq!(v.today, "1ч 15мин");
    }

    #[test]
    fn label_color_ids_and_tags_from_catalog() {
        let web = project("Website", "#8b5cf6");
        let reports = task(&web, "Reports");
        let (design, backend) = (tag("design"), tag("backend"));
        let c = Catalog {
            projects: vec![web.clone()],
            tasks: vec![reports.clone()],
            // Catalog order (by name).
            tags: vec![backend.clone(), design.clone()],
        };
        let mut s = status(SessionState::Running, DAY0, 0, 0);
        s.template = Some(EntryTemplate {
            description: String::new(),
            project_id: Some(web.id),
            task_id: Some(reports.id),
            tag_ids: vec![design.id, backend.id],
            billable: true,
        });
        let v = timer_view(&s, &EntryTemplate::default(), &c, &prefs(), DAY0);
        assert_eq!(v.project_label, "Website › Reports");
        assert_eq!(v.project_color, Color::from_rgb_u8(0x8b, 0x5c, 0xf6));
        assert_eq!(v.project_id, web.id.to_string());
        assert_eq!(v.task_id, reports.id.to_string());
        assert_eq!(v.tags_label, "backend, design");
        assert!(v.billable);

        // Project without task.
        s.template.as_mut().unwrap().task_id = None;
        let v = timer_view(&s, &EntryTemplate::default(), &c, &prefs(), DAY0);
        assert_eq!(v.project_label, "Website");
        assert_eq!(v.task_id, "");
    }

    #[test]
    fn active_template_falls_back_to_current_entry() {
        assert_eq!(active_template(&TimerStatus::default()), None);
        let mut s = status(SessionState::Paused, DAY0, 0, 0);
        assert_eq!(active_template(&s).unwrap().description, "Reports page");
        s.template = None;
        s.current = Some(tt_domain::TimeEntry {
            id: new_id(),
            session_id: None,
            description: "from entry".into(),
            start: DAY0,
            end: Some(DAY0 + 1),
            billable: false,
            project_id: None,
            task_id: None,
            tag_ids: vec![],
            created_at: 0,
            updated_at: 0,
        });
        assert_eq!(active_template(&s).unwrap().description, "from entry");
    }
}
