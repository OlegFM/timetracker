//! Picker rows (project/task and tags), search and selection helpers.

use slint::{Color, SharedString};
use tt_domain::{EntryTemplate, Id, Project, Tag, Task};

use crate::{ProjectOption, TagOption};

/// Between project and task in option labels.
pub const SEPARATOR: &str = " › ";

/// Drops the project, task and tags of `t` that are not in `catalog` (deleted meanwhile);
/// without its project the task goes too.
pub fn retain_known(t: &mut EntryTemplate, catalog: &Catalog) {
    if t.project_id.is_some_and(|id| catalog.project(id).is_none()) {
        t.project_id = None;
        t.task_id = None;
    }
    if t.task_id.is_some_and(|id| catalog.task(id).is_none()) {
        t.task_id = None;
    }
    t.tag_ids
        .retain(|id| catalog.tags.iter().any(|tag| tag.id == *id));
}

/// Colors given to projects created from the picker, in turn.
pub const PALETTE: [&str; 12] = [
    "#ef4444", "#f97316", "#f59e0b", "#84cc16", "#22c55e", "#14b8a6", "#06b6d4", "#3b82f6",
    "#6366f1", "#8b5cf6", "#d946ef", "#ec4899",
];

/// Catalog snapshot for labels and options; includes archived projects and done tasks so
/// that existing entries still resolve.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub tags: Vec<Tag>,
}

impl Catalog {
    pub fn project(&self, id: Id) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == id)
    }

    pub fn task(&self, id: Id) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }
}

/// Color of the `n`-th created project.
pub fn palette_color(n: usize) -> &'static str {
    PALETTE[n % PALETTE.len()]
}

/// "Project › Task" or "Project".
pub fn option_label(project: &Project, task: Option<&Task>) -> String {
    match task {
        Some(t) => format!("{}{SEPARATOR}{}", project.name, t.name),
        None => project.name.clone(),
    }
}

/// `#rrggbb` as a color; anything else is transparent.
pub fn hex_color(s: &str) -> Color {
    let rgb = s
        .strip_prefix('#')
        .filter(|h| h.len() == 6)
        .and_then(|h| u32::from_str_radix(h, 16).ok());
    match rgb {
        Some(v) => Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8),
        None => Color::default(),
    }
}

/// Empty or invalid text is `None`.
pub fn parse_id(s: &str) -> Option<Id> {
    Id::parse_str(s).ok()
}

/// Non-archived projects by name, each followed by its open tasks by name.
pub fn project_options(c: &Catalog) -> Vec<ProjectOption> {
    let mut projects: Vec<&Project> = c
        .projects
        .iter()
        .filter(|p| p.archived_at.is_none())
        .collect();
    projects.sort_by_key(|p| p.name.to_lowercase());
    let mut out = Vec::new();
    for p in projects {
        let color = hex_color(&p.color);
        out.push(ProjectOption {
            id: p.id.to_string().into(),
            label: option_label(p, None).into(),
            color,
            project_id: p.id.to_string().into(),
            task_id: SharedString::new(),
        });
        let mut tasks: Vec<&Task> = c
            .tasks
            .iter()
            .filter(|t| t.project_id == p.id && t.done_at.is_none())
            .collect();
        tasks.sort_by_key(|t| t.name.to_lowercase());
        out.extend(tasks.into_iter().map(|t| ProjectOption {
            id: t.id.to_string().into(),
            label: option_label(p, Some(t)).into(),
            color,
            project_id: p.id.to_string().into(),
            task_id: t.id.to_string().into(),
        }));
    }
    out
}

/// All tags (catalog order), marked if in `selected`.
pub fn tag_options(c: &Catalog, selected: &[Id]) -> Vec<TagOption> {
    c.tags
        .iter()
        .map(|t| TagOption {
            id: t.id.to_string().into(),
            name: t.name.as_str().into(),
            selected: selected.contains(&t.id),
        })
        .collect()
}

/// Items whose key contains the trimmed query (case-insensitive), exact matches first.
fn filter_by<T: Clone>(items: &[T], query: &str, key: impl Fn(&T) -> &str) -> Vec<T> {
    let q = query.trim().to_lowercase();
    let (mut exact, mut partial) = (Vec::new(), Vec::new());
    for item in items {
        let k = key(item).to_lowercase();
        if k == q {
            exact.push(item.clone());
        } else if k.contains(&q) {
            partial.push(item.clone());
        }
    }
    exact.append(&mut partial);
    exact
}

fn creatable_by<T>(items: &[T], query: &str, key: impl Fn(&T) -> &str) -> bool {
    let q = query.trim().to_lowercase();
    !q.is_empty() && !items.iter().any(|i| key(i).to_lowercase() == q)
}

pub fn filter_options(items: &[ProjectOption], query: &str) -> Vec<ProjectOption> {
    filter_by(items, query, |o| o.label.as_str())
}

/// The first `limit` options (a very long list is searched instead of scrolled).
pub fn limit_options(items: &[ProjectOption], limit: i32) -> Vec<ProjectOption> {
    items
        .iter()
        .take(usize::try_from(limit).unwrap_or_default())
        .cloned()
        .collect()
}

/// Index of the option with `id` (the current choice); -1 if absent or `id` is empty,
/// so that no row (in particular not "No project") is preselected.
pub fn option_index(items: &[ProjectOption], id: &str) -> i32 {
    if id.is_empty() {
        return -1;
    }
    items
        .iter()
        .position(|o| o.id == id)
        .map_or(-1, |i| i as i32)
}

pub fn option_creatable(items: &[ProjectOption], query: &str) -> bool {
    creatable_by(items, query, |o| o.label.as_str())
}

pub fn filter_tags(items: &[TagOption], query: &str) -> Vec<TagOption> {
    filter_by(items, query, |t| t.name.as_str())
}

pub fn tag_creatable(items: &[TagOption], query: &str) -> bool {
    creatable_by(items, query, |t| t.name.as_str())
}

/// `items` with `selected` set exactly for `ids` (for a selection other than the timer's).
pub fn select_tags(items: &[TagOption], ids: &[SharedString]) -> Vec<TagOption> {
    items
        .iter()
        .map(|t| TagOption {
            selected: ids.contains(&t.id),
            ..t.clone()
        })
        .collect()
}

/// Names of the tags in `ids`, in the order of `items`, joined with ", ".
pub fn selected_names(items: &[TagOption], ids: &[SharedString]) -> String {
    let names: Vec<&str> = items
        .iter()
        .filter(|t| ids.contains(&t.id))
        .map(|t| t.name.as_str())
        .collect();
    names.join(", ")
}

/// Selected tag ids with `id` toggled (an unknown id is added).
pub fn toggle_tag(items: &[TagOption], id: &str) -> Vec<SharedString> {
    let mut ids: Vec<SharedString> = items
        .iter()
        .filter(|t| t.selected)
        .map(|t| t.id.clone())
        .collect();
    match ids.iter().position(|s| s == id) {
        Some(i) => {
            ids.remove(i);
        }
        None => ids.push(id.into()),
    }
    ids
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tt_domain::new_id;

    pub fn project(name: &str, color: &str) -> Project {
        Project {
            id: new_id(),
            name: name.into(),
            color: color.into(),
            client_id: None,
            is_billable: false,
            billable_rate: None,
            estimated_time: None,
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    pub fn task(p: &Project, name: &str) -> Task {
        Task {
            id: new_id(),
            project_id: p.id,
            name: name.into(),
            estimated_time: None,
            done_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    pub fn tag(name: &str) -> Tag {
        Tag {
            id: new_id(),
            name: name.into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn template_keeps_only_ids_of_the_catalog() {
        let p = project("p", "#000000");
        let t = task(&p, "t");
        let keep = tag("keep");
        let catalog = Catalog {
            projects: vec![p.clone()],
            tasks: vec![t.clone()],
            tags: vec![keep.clone()],
        };
        let mut draft = EntryTemplate {
            description: "d".into(),
            project_id: Some(p.id),
            task_id: Some(t.id),
            tag_ids: vec![new_id(), keep.id],
            billable: true,
        };
        retain_known(&mut draft, &catalog);
        assert_eq!((draft.project_id, draft.task_id), (Some(p.id), Some(t.id)));
        assert_eq!(draft.tag_ids, vec![keep.id]);

        // A deleted task: the project stays.
        draft.task_id = Some(new_id());
        retain_known(&mut draft, &catalog);
        assert_eq!((draft.project_id, draft.task_id), (Some(p.id), None));

        // A deleted project takes its task along.
        draft.project_id = Some(new_id());
        draft.task_id = Some(t.id);
        retain_known(&mut draft, &catalog);
        assert_eq!((draft.project_id, draft.task_id), (None, None));
        assert_eq!((draft.description.as_str(), draft.billable), ("d", true));
    }

    fn labels(items: &[ProjectOption]) -> Vec<&str> {
        items.iter().map(|o| o.label.as_str()).collect()
    }

    fn opt(label: &str) -> ProjectOption {
        ProjectOption {
            label: label.into(),
            ..Default::default()
        }
    }

    #[test]
    fn options_skip_archived_projects_and_done_tasks_and_sort_by_name() {
        let web = project("website", "#8b5cf6");
        let mut old = project("Archive me", "#000000");
        old.archived_at = Some(1);
        let app = project("App", "#3b82f6");
        let reports = task(&web, "Reports");
        let mut done = task(&web, "Done one");
        done.done_at = Some(1);
        let api = task(&web, "api");
        let hidden = task(&old, "Hidden");
        let c = Catalog {
            projects: vec![web.clone(), old, app.clone()],
            tasks: vec![reports.clone(), done, api, hidden],
            tags: vec![],
        };
        let opts = project_options(&c);
        assert_eq!(
            labels(&opts),
            ["App", "website", "website › api", "website › Reports"]
        );
        let r = &opts[3];
        assert_eq!(r.id, reports.id.to_string());
        assert_eq!(r.project_id, web.id.to_string());
        assert_eq!(r.task_id, reports.id.to_string());
        assert_eq!(r.color, Color::from_rgb_u8(0x8b, 0x5c, 0xf6));
        assert_eq!(opts[0].id, app.id.to_string());
        assert_eq!(opts[0].task_id, "");
    }

    #[test]
    fn hex_color_parses_rrggbb_only() {
        assert_eq!(hex_color("#22c55e"), Color::from_rgb_u8(0x22, 0xc5, 0x5e));
        assert_eq!(hex_color("22c55e"), Color::default());
        assert_eq!(hex_color("#fff"), Color::default());
    }

    #[test]
    fn limit_keeps_the_first_options() {
        let items = [opt("a"), opt("b"), opt("c")];
        assert_eq!(labels(&limit_options(&items, 2)), ["a", "b"]);
        assert_eq!(labels(&limit_options(&items, 9)), ["a", "b", "c"]);
        assert!(limit_options(&items, 0).is_empty());
        assert!(limit_options(&items, -1).is_empty());
    }

    #[test]
    fn filter_is_case_insensitive_substring_with_exact_first() {
        let items = [opt("Website › Design"), opt("App"), opt("design")];
        assert_eq!(
            labels(&filter_options(&items, "DESIGN")),
            ["design", "Website › Design"]
        );
        assert_eq!(
            labels(&filter_options(&items, " ")),
            ["Website › Design", "App", "design"]
        );
        assert!(filter_options(&items, "zzz").is_empty());
        let tags = [tag_opt("Back-end", false), tag_opt("ДИЗАЙН", false)];
        let found = filter_tags(&tags, "дизайн");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "ДИЗАЙН");
    }

    #[test]
    fn option_index_finds_current_choice_only() {
        let items = [
            ProjectOption {
                id: "p".into(),
                ..opt("Website")
            },
            ProjectOption {
                id: "t".into(),
                ..opt("Website › Reports")
            },
        ];
        assert_eq!(option_index(&items, "t"), 1);
        assert_eq!(option_index(&items, "p"), 0);
        // No current choice: nothing preselected (not the "No project" row).
        assert_eq!(option_index(&items, ""), -1);
        // Current choice not pickable (archived project / done task).
        assert_eq!(option_index(&items, "gone"), -1);
    }

    #[test]
    fn creatable_needs_text_without_exact_match() {
        let items = [opt("Website › Design"), opt("App")];
        assert!(option_creatable(&items, "Design"));
        assert!(!option_creatable(&items, " app "));
        assert!(!option_creatable(&items, "  "));
        let tags = [tag_opt("urgent", false)];
        assert!(!tag_creatable(&tags, "URGENT"));
        assert!(tag_creatable(&tags, "urge"));
    }

    fn tag_opt(name: &str, selected: bool) -> TagOption {
        TagOption {
            id: name.into(),
            name: name.into(),
            selected,
        }
    }

    #[test]
    fn toggle_tag_adds_and_removes() {
        let items = [tag_opt("a", true), tag_opt("b", false), tag_opt("c", true)];
        assert_eq!(toggle_tag(&items, "b"), ["a", "c", "b"]);
        assert_eq!(toggle_tag(&items, "a"), ["c"]);
        assert_eq!(toggle_tag(&items, "new"), ["a", "c", "new"]);
    }

    #[test]
    fn select_tags_and_names_follow_given_ids() {
        let items = [tag_opt("a", true), tag_opt("b", false), tag_opt("c", false)];
        let ids = [SharedString::from("c"), SharedString::from("b")];
        let marked = select_tags(&items, &ids);
        assert_eq!(
            marked.iter().map(|t| t.selected).collect::<Vec<_>>(),
            [false, true, true]
        );
        assert_eq!(selected_names(&items, &ids), "b, c");
        assert_eq!(selected_names(&items, &[]), "");
        // Unknown ids (e.g. a tag deleted meanwhile) are ignored.
        assert_eq!(selected_names(&items, &["zz".into()]), "");
    }

    #[test]
    fn tag_options_mark_selection() {
        let (a, b) = (tag("a"), tag("b"));
        let c = Catalog {
            tags: vec![a.clone(), b],
            ..Default::default()
        };
        let opts = tag_options(&c, &[a.id]);
        assert_eq!(
            opts.iter().map(|t| t.selected).collect::<Vec<_>>(),
            [true, false]
        );
    }

    #[test]
    fn palette_rotates() {
        assert_eq!(palette_color(0), PALETTE[0]);
        assert_eq!(palette_color(13), PALETTE[1]);
    }

    #[test]
    fn parse_id_rejects_empty() {
        let id = new_id();
        assert_eq!(parse_id(&id.to_string()), Some(id));
        assert_eq!(parse_id(""), None);
    }
}
