//! Creates a demo database for manual checks of the UI.
//!
//! ```text
//! cargo run -p tt-app --example seed -- <path-to-new.db> [--running]
//! cargo run -p tt-app -- --db <path-to-new.db>                   # any build
//! $env:TT_DB_PATH = "<path-to-new.db>"; cargo run -p tt-app      # debug builds only
//! ```
//!
//! Two clients, five projects (colors, rates, one estimate), tasks, tags and about 300
//! closed entries over the last 90 days; with `--running` the timer is left running
//! (memory measurements, `docs/perf.md`). The numbers come from a small fixed-seed
//! generator, so every run produces the same data shape. Entries stay inside one local day
//! (08:00 to 20:00), so none is cut at midnight. Refuses to touch an existing file.

use std::path::PathBuf;

use anyhow::{Context, bail};
use chrono::{Datelike, Days, Weekday};
use tt_app::app::AppContext;
use tt_app::vm::entries::local_millis;
use tt_domain::{EntryTemplate, Id, Millis, local_date};
use tt_services::{Clock, NewProject, default_billable};

const DAYS: u64 = 90;
const MINUTE: Millis = 60_000;

/// Linear congruential generator (Knuth's MMIX constants): enough for demo data.
struct Lcg(u64);

impl Lcg {
    /// A number in `0..n`.
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(n)) as u32
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }
}

struct Seeded {
    project: Id,
    tasks: Vec<Id>,
    descriptions: &'static [&'static str],
    /// Relative share of the time entries.
    weight: u32,
}

fn main() -> anyhow::Result<()> {
    let path: PathBuf = std::env::args_os()
        .nth(1)
        .context("usage: seed <path-to-new.db>")?
        .into();
    if path.exists() {
        bail!("{} already exists; refusing to touch it", path.display());
    }
    let running = std::env::args().skip(2).any(|a| a == "--running");
    let mut ctx = AppContext::open(path.clone())?;
    let c = &mut ctx;
    let tz = c.settings.format.tz;

    let acme = c.catalog.create_client(&mut c.db, "Acme Corp")?;
    let globex = c.catalog.create_client(&mut c.db, "Globex")?;
    let tags: Vec<Id> = ["design", "dev", "meeting", "review"]
        .iter()
        .map(|n| c.catalog.create_tag(&mut c.db, n).map(|t| t.id))
        .collect::<Result<_, _>>()?;

    // (name, color, client, billable rate in cents per hour, estimate in hours, tasks,
    // descriptions, weight)
    type Spec = (
        &'static str,
        &'static str,
        Option<Id>,
        Option<i64>,
        Option<i64>,
        &'static [&'static str],
        &'static [&'static str],
        u32,
    );
    let specs: [Spec; 5] = [
        (
            "Website redesign",
            "#3b82f6",
            Some(acme.id),
            Some(6_000),
            Some(120),
            &["Wireframes", "Homepage", "Checkout"],
            &[
                "Layout review",
                "Header and footer",
                "Responsive fixes",
                "Style guide",
            ],
            6,
        ),
        (
            "Mobile app",
            "#22c55e",
            Some(acme.id),
            Some(7_500),
            None,
            &["Login flow", "Push notifications"],
            &[
                "Fix crash on start",
                "Sync engine",
                "Release build",
                "Profile screen",
            ],
            5,
        ),
        (
            "Internal tools",
            "#f59e0b",
            Some(globex.id),
            None,
            None,
            &["Reports", "Deploy scripts"],
            &["Nightly report", "Refactor importer", "Write docs"],
            3,
        ),
        (
            "Marketing site",
            "#ec4899",
            Some(globex.id),
            Some(5_000),
            None,
            &["Landing page"],
            &["Copy edits", "Blog post", "Analytics setup"],
            2,
        ),
        (
            "Admin and meetings",
            "#8b5cf6",
            None,
            None,
            None,
            &[],
            &["Weekly sync", "Planning", "Email and invoices"],
            2,
        ),
    ];

    let mut seeded = Vec::new();
    let mut billable = Vec::new();
    for (name, color, client_id, rate, estimate_h, tasks, descriptions, weight) in specs {
        let project = c.catalog.create_project(
            &mut c.db,
            NewProject {
                name: name.into(),
                color: color.into(),
                client_id,
                is_billable: rate.is_some(),
                billable_rate: rate,
                estimated_time: estimate_h.map(|h| h * 3600),
            },
        )?;
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(c.catalog.create_task(&mut c.db, project.id, task, None)?.id);
        }
        billable.push(default_billable(Some(&project)));
        seeded.push(Seeded {
            project: project.id,
            tasks: ids,
            descriptions,
            weight,
        });
    }
    let weights: Vec<u32> = seeded.iter().map(|s| s.weight).collect();
    let total_weight: u32 = weights.iter().sum();

    let now = c.timer.clock().now();
    let today = local_date(now, tz);
    let mut rng = Lcg(0x5EED_0001);
    let mut count = 0;
    for back in (0..=DAYS).rev() {
        let day = today - Days::new(back);
        // Weekends are mostly free; some weekdays are light.
        let weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
        if weekend && rng.below(2) != 0 {
            continue;
        }
        let entries = if weekend { 2 } else { 2 + rng.below(5) };
        let mut minute = 8 * 60 + rng.below(90); // first start, minutes since midnight
        for _ in 0..entries {
            let length = 20 + rng.below(130);
            if minute + length > 20 * 60 {
                break;
            }
            let start = local_millis(day, minute / 60, minute % 60, tz);
            let end = start + Millis::from(length) * MINUTE;
            minute += length + 5 + rng.below(40);
            if end > now {
                continue; // today, later than now
            }

            // Pick a project by weight.
            let mut roll = rng.below(total_weight);
            let i = weights
                .iter()
                .position(|w| {
                    let hit = roll < *w;
                    roll = roll.saturating_sub(*w);
                    hit
                })
                .unwrap_or(0);
            let s = &seeded[i];
            let task = (!s.tasks.is_empty() && rng.below(3) != 0).then(|| *rng.pick(&s.tasks));
            let tag_ids = if rng.below(2) == 0 {
                vec![*rng.pick(&tags)]
            } else {
                Vec::new()
            };
            let template = EntryTemplate {
                description: (*rng.pick(s.descriptions)).to_owned(),
                project_id: Some(s.project),
                task_id: task,
                tag_ids,
                billable: billable[i],
            };
            c.entries
                .create_manual(&mut c.db, template, start, end, tz, false)?;
            count += 1;
        }
    }
    if running {
        let s = &seeded[0];
        let template = EntryTemplate {
            description: s.descriptions[0].to_owned(),
            project_id: Some(s.project),
            task_id: None,
            tag_ids: Vec::new(),
            billable: billable[0],
        };
        c.timer.start(&mut c.db, template, tz)?;
    }
    println!(
        "Created {} with 2 clients, {} projects, {} tags and {count} time entries.",
        path.display(),
        seeded.len(),
        tags.len()
    );
    if running {
        println!("The timer is running.");
    }
    Ok(())
}
