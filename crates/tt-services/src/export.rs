//! CSV and XLSX export of reports and entries. Header texts come from the UI layer.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use rust_xlsxwriter::{Format, Workbook, Worksheet};
use tt_domain::{
    Client, FormatPrefs, GroupKey, Id, Project, ReportResult, Tag, Task, TimeEntry, format_date,
    format_duration, format_money, format_time, local_date,
};

use crate::error::{Result, ServiceError};

/// Localized texts used as headers and placeholders.
#[derive(Debug, Clone, Default)]
pub struct ExportLabels {
    pub group: String,
    pub subgroup: String,
    pub duration: String,
    pub cost: String,
    pub total: String,
    pub no_project: String,
    pub no_task: String,
    pub no_client: String,
    pub no_tag: String,
    pub yes: String,
    pub no: String,
    pub description: String,
    pub project: String,
    pub task: String,
    pub client: String,
    pub tags: String,
    pub billable: String,
    pub start: String,
    pub end: String,
    pub date: String,
}

pub struct Lookup<'a> {
    pub projects: &'a HashMap<Id, Project>,
    pub tasks: &'a HashMap<Id, Task>,
    pub clients: &'a HashMap<Id, Client>,
    pub tags: &'a HashMap<Id, Tag>,
}

type Names<'a> = &'a dyn Fn(&GroupKey) -> String;

fn io_err(e: impl std::error::Error + Send + Sync + 'static) -> ServiceError {
    ServiceError::Io(std::io::Error::other(e))
}

/// One summary line. `sub` is empty for a level-1 row and the total.
struct Line {
    group: String,
    sub: String,
    seconds: i64,
    cost: i64,
}

fn summary_lines(r: &ReportResult, names: Names, labels: &ExportLabels) -> Vec<Line> {
    let mut out = Vec::new();
    for g in &r.groups {
        let group = names(&g.key);
        out.push(Line {
            group: group.clone(),
            sub: String::new(),
            seconds: g.seconds,
            cost: g.cost_cents,
        });
        for c in &g.children {
            out.push(Line {
                group: group.clone(),
                sub: names(&c.key),
                seconds: c.seconds,
                cost: c.cost_cents,
            });
        }
    }
    out.push(Line {
        group: labels.total.clone(),
        sub: String::new(),
        seconds: r.total_seconds,
        cost: r.total_cost_cents,
    });
    out
}

/// Cells of a closed entry, in export column order (duration kept numeric).
struct EntryRow {
    date: String,
    start: String,
    end: String,
    seconds: i64,
    text: [String; 6], // description, project, task, client, tags, billable
}

fn entry_headers(l: &ExportLabels) -> [&str; 10] {
    [
        &l.date,
        &l.start,
        &l.end,
        &l.duration,
        &l.description,
        &l.project,
        &l.task,
        &l.client,
        &l.tags,
        &l.billable,
    ]
}

/// Rows of the closed entries; running ones are skipped.
fn entry_rows(
    entries: &[TimeEntry],
    lk: &Lookup,
    l: &ExportLabels,
    p: &FormatPrefs,
) -> Vec<EntryRow> {
    entries
        .iter()
        .filter_map(|e| {
            let end = e.end?;
            let project = e.project_id.and_then(|id| lk.projects.get(&id));
            let client = project
                .and_then(|pr| pr.client_id)
                .and_then(|id| lk.clients.get(&id));
            let task = e.task_id.and_then(|id| lk.tasks.get(&id));
            let tags: Vec<&str> = e
                .tag_ids
                .iter()
                .filter_map(|id| lk.tags.get(id))
                .map(|t| t.name.as_str())
                .collect();
            Some(EntryRow {
                date: format_date(local_date(e.start, p.tz), p.date),
                start: format_time(e.start, p),
                end: format_time(end, p),
                seconds: (end - e.start).max(0) / 1000,
                text: [
                    e.description.clone(),
                    project.map_or(&l.no_project, |x| &x.name).clone(),
                    task.map_or(&l.no_task, |x| &x.name).clone(),
                    client.map_or(&l.no_client, |x| &x.name).clone(),
                    tags.join(", "),
                    if e.billable { &l.yes } else { &l.no }.clone(),
                ],
            })
        })
        .collect()
}

/// Defuses spreadsheet formula injection: text starting with a formula trigger gets a `'` prefix.
fn safe(text: &str) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

fn write_bom(w: &mut impl Write) -> Result<()> {
    w.write_all(&[0xEF, 0xBB, 0xBF]).map_err(ServiceError::Io)
}

pub fn export_report_csv(
    mut w: impl Write,
    r: &ReportResult,
    names: Names,
    labels: &ExportLabels,
    prefs: &FormatPrefs,
) -> Result<()> {
    write_bom(&mut w)?;
    let with_sub = r.groups.iter().any(|g| !g.children.is_empty());
    let mut out = csv::Writer::from_writer(w);
    let mut record = |group: &str, sub: &str, duration: &str, cost: &str| {
        if with_sub {
            out.write_record([group, sub, duration, cost])
        } else {
            out.write_record([group, duration, cost])
        }
    };
    record(
        &labels.group,
        &labels.subgroup,
        &labels.duration,
        &labels.cost,
    )
    .map_err(io_err)?;
    for l in summary_lines(r, names, labels) {
        record(
            &safe(&l.group),
            &safe(&l.sub),
            &format_duration(l.seconds, prefs),
            &format_money(l.cost, prefs),
        )
        .map_err(io_err)?;
    }
    out.flush().map_err(ServiceError::Io)
}

pub fn export_entries_csv(
    mut w: impl Write,
    entries: &[TimeEntry],
    lookup: &Lookup,
    labels: &ExportLabels,
    prefs: &FormatPrefs,
) -> Result<()> {
    write_bom(&mut w)?;
    let mut out = csv::Writer::from_writer(w);
    out.write_record(entry_headers(labels)).map_err(io_err)?;
    for r in entry_rows(entries, lookup, labels, prefs) {
        let duration = format_duration(r.seconds, prefs);
        let [description, project, task, client, tags, billable] = &r.text;
        out.write_record([
            &r.date,
            &r.start,
            &r.end,
            &duration,
            &safe(description),
            &safe(project),
            &safe(task),
            &safe(client),
            &safe(tags),
            billable,
        ])
        .map_err(io_err)?;
    }
    out.flush().map_err(ServiceError::Io)
}

fn hours(seconds: i64) -> f64 {
    seconds as f64 / 3600.0
}

fn header(sheet: &mut Worksheet, cells: &[&str], bold: &Format) -> Result<()> {
    for (c, text) in cells.iter().enumerate() {
        sheet
            .write_string_with_format(0, c as u16, *text, bold)
            .map_err(io_err)?;
    }
    Ok(())
}

pub fn export_report_xlsx(
    path: &Path,
    r: &ReportResult,
    names: Names,
    entries: &[TimeEntry],
    lookup: &Lookup,
    labels: &ExportLabels,
    prefs: &FormatPrefs,
) -> Result<()> {
    let bold = Format::new().set_bold();
    let num = Format::new().set_num_format("0.00");
    let mut book = Workbook::new();

    let sheet = book.add_worksheet();
    sheet.set_name("Summary").map_err(io_err)?;
    header(
        sheet,
        &[
            &labels.group,
            &labels.duration,
            &labels.duration,
            &labels.cost,
        ],
        &bold,
    )?;
    for (i, l) in summary_lines(r, names, labels).iter().enumerate() {
        let row = i as u32 + 1;
        let name = if l.sub.is_empty() {
            l.group.clone()
        } else {
            format!("  {}", l.sub)
        };
        sheet.write_string(row, 0, name).map_err(io_err)?;
        sheet
            .write_number_with_format(row, 1, hours(l.seconds), &num)
            .map_err(io_err)?;
        sheet
            .write_string(row, 2, format_duration(l.seconds, prefs))
            .map_err(io_err)?;
        sheet
            .write_number_with_format(row, 3, l.cost as f64 / 100.0, &num)
            .map_err(io_err)?;
    }

    let sheet = book.add_worksheet();
    sheet.set_name("Entries").map_err(io_err)?;
    header(sheet, &entry_headers(labels), &bold)?;
    for (i, r) in entry_rows(entries, lookup, labels, prefs)
        .iter()
        .enumerate()
    {
        let row = i as u32 + 1;
        for (c, text) in [&r.date, &r.start, &r.end].into_iter().enumerate() {
            sheet.write_string(row, c as u16, text).map_err(io_err)?;
        }
        sheet
            .write_number_with_format(row, 3, hours(r.seconds), &num)
            .map_err(io_err)?;
        for (c, text) in r.text.iter().enumerate() {
            sheet
                .write_string(row, c as u16 + 4, text)
                .map_err(io_err)?;
        }
    }
    book.save(path).map_err(io_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tt_domain::{GroupRow, new_id};

    fn labels() -> ExportLabels {
        let s = |x: &str| x.to_owned();
        ExportLabels {
            group: s("Group"),
            subgroup: s("Subgroup"),
            duration: s("Duration"),
            cost: s("Cost"),
            total: s("Total"),
            no_project: s("No project"),
            no_task: s("No task"),
            no_client: s("No client"),
            no_tag: s("No tag"),
            yes: s("Yes"),
            no: s("No"),
            description: s("Description"),
            project: s("Project"),
            task: s("Task"),
            client: s("Client"),
            tags: s("Tags"),
            billable: s("Billable"),
            start: s("Start"),
            end: s("End"),
            date: s("Date"),
        }
    }

    fn row(key: GroupKey, seconds: i64, cost: i64, children: Vec<GroupRow>) -> GroupRow {
        GroupRow {
            key,
            seconds,
            cost_cents: cost,
            children,
        }
    }

    fn report(a: Id, b: Id) -> ReportResult {
        ReportResult {
            total_seconds: 5400,
            total_cost_cents: 1500,
            groups: vec![
                row(
                    GroupKey::Id(a),
                    3600,
                    1000,
                    vec![row(GroupKey::Bool(true), 3600, 1000, vec![])],
                ),
                row(GroupKey::Id(b), 1800, 500, vec![]),
            ],
            buckets: vec![],
        }
    }

    fn names(a: Id, b: Id) -> impl Fn(&GroupKey) -> String {
        move |k| match k {
            GroupKey::Id(i) if *i == a => "Alpha".into(),
            GroupKey::Id(i) if *i == b => "Beta".into(),
            GroupKey::Bool(true) => "billable".into(),
            _ => "?".into(),
        }
    }

    fn strip_bom(bytes: &[u8]) -> &[u8] {
        bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).expect("BOM")
    }

    fn entry(description: &str, end: Option<i64>) -> TimeEntry {
        TimeEntry {
            id: new_id(),
            session_id: None,
            description: description.into(),
            start: 0,
            end,
            billable: true,
            project_id: None,
            task_id: None,
            tag_ids: vec![],
            created_at: 0,
            updated_at: 0,
        }
    }

    fn empty_lookup<R>(f: impl FnOnce(&Lookup) -> R) -> R {
        let (p, t, c, g) = Default::default();
        f(&Lookup {
            projects: &p,
            tasks: &t,
            clients: &c,
            tags: &g,
        })
    }

    #[test]
    fn csv_report_round_trip() {
        let (a, b) = (new_id(), new_id());
        let mut buf = Vec::new();
        export_report_csv(
            &mut buf,
            &report(a, b),
            &names(a, b),
            &labels(),
            &FormatPrefs::default(),
        )
        .unwrap();
        let mut rd = csv::Reader::from_reader(strip_bom(&buf));
        let headers: Vec<String> = rd.headers().unwrap().iter().map(str::to_owned).collect();
        assert_eq!(headers, ["Group", "Subgroup", "Duration", "Cost"]);
        let rows: Vec<Vec<String>> = rd
            .records()
            .map(|r| r.unwrap().iter().map(str::to_owned).collect())
            .collect();
        assert_eq!(
            rows,
            [
                ["Alpha", "", "1:00", "$10.00"],
                ["Alpha", "billable", "1:00", "$10.00"],
                ["Beta", "", "0:30", "$5.00"],
                ["Total", "", "1:30", "$15.00"],
            ]
        );
    }

    #[test]
    fn csv_report_without_children_has_no_subgroup_column() {
        let a = new_id();
        let mut r = report(a, a);
        r.groups.truncate(2);
        r.groups[0].children.clear();
        let mut buf = Vec::new();
        export_report_csv(
            &mut buf,
            &r,
            &names(a, a),
            &labels(),
            &FormatPrefs::default(),
        )
        .unwrap();
        let mut rd = csv::Reader::from_reader(strip_bom(&buf));
        assert_eq!(rd.headers().unwrap().len(), 3);
    }

    #[test]
    fn csv_starts_with_bom() {
        let mut buf = Vec::new();
        empty_lookup(|lk| {
            export_entries_csv(&mut buf, &[], lk, &labels(), &FormatPrefs::default()).unwrap();
        });
        assert_eq!(&buf[..3], [0xEF, 0xBB, 0xBF]);
        let mut buf = Vec::new();
        let a = new_id();
        export_report_csv(
            &mut buf,
            &report(a, a),
            &names(a, a),
            &labels(),
            &FormatPrefs::default(),
        )
        .unwrap();
        assert_eq!(&buf[..3], [0xEF, 0xBB, 0xBF]);
    }

    #[test]
    fn csv_escapes_quotes_and_newlines_in_description() {
        let text = "say \"hi\",\nsecond line";
        let entries = [entry(text, Some(3_600_000)), entry("running", None)];
        let mut buf = Vec::new();
        empty_lookup(|lk| {
            export_entries_csv(&mut buf, &entries, lk, &labels(), &FormatPrefs::default()).unwrap();
        });
        let mut rd = csv::Reader::from_reader(strip_bom(&buf));
        let recs: Vec<csv::StringRecord> = rd.records().map(|r| r.unwrap()).collect();
        assert_eq!(recs.len(), 1, "running entry is skipped");
        assert_eq!(&recs[0][4], text);
        assert_eq!(&recs[0][3], "1:00");
        assert_eq!(&recs[0][5], "No project");
        assert_eq!(&recs[0][9], "Yes");
    }

    #[test]
    fn xlsx_has_two_sheets() {
        let (a, b) = (new_id(), new_id());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.xlsx");
        empty_lookup(|lk| {
            export_report_xlsx(
                &path,
                &report(a, b),
                &names(a, b),
                &[entry("work", Some(1_800_000))],
                lk,
                &labels(),
                &FormatPrefs::default(),
            )
            .unwrap();
        });
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..2], b"PK");
        let has = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
        assert!(has("xl/worksheets/sheet1.xml"));
        assert!(has("xl/worksheets/sheet2.xml"));
    }

    #[test]
    fn csv_neutralizes_formula_cells() {
        let proj = Project {
            id: new_id(),
            name: "@SUM(A1)".into(),
            color: "#000000".into(),
            client_id: None,
            is_billable: false,
            billable_rate: None,
            estimated_time: None,
            archived_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let projects = HashMap::from([(proj.id, proj.clone())]);
        let (t, c, g) = Default::default();
        let lk = Lookup {
            projects: &projects,
            tasks: &t,
            clients: &c,
            tags: &g,
        };
        let mut e = entry("=1+1", Some(3_600_000));
        e.project_id = Some(proj.id);
        let mut buf = Vec::new();
        export_entries_csv(&mut buf, &[e], &lk, &labels(), &FormatPrefs::default()).unwrap();
        let mut rd = csv::Reader::from_reader(strip_bom(&buf));
        let rec = rd.records().next().unwrap().unwrap();
        assert_eq!(&rec[4], "'=1+1");
        assert_eq!(&rec[5], "'@SUM(A1)");

        let a = new_id();
        let mut buf = Vec::new();
        export_report_csv(
            &mut buf,
            &report(a, a),
            &|_: &GroupKey| "+cmd".to_owned(),
            &labels(),
            &FormatPrefs::default(),
        )
        .unwrap();
        let mut rd = csv::Reader::from_reader(strip_bom(&buf));
        let rec = rd.records().next().unwrap().unwrap();
        assert_eq!(&rec[0], "'+cmd");
        assert_eq!(&rec[1], "");
    }
}
