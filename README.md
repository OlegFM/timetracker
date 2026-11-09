# TimeTracker

A lightweight time tracker for Windows: a native window, a floating mini timer and a
tray icon. Everything stays on your computer in one SQLite file — no account, no cloud.
The behaviour follows [solidtime](https://www.solidtime.io/) (core features only); the
code is our own.

Written in Rust with [Slint](https://slint.dev). In the tray with the timer running it
uses about 5–6 MB of memory and practically no CPU (see [docs/perf.md](docs/perf.md)).

## Features (v1)

- **Timer** on every screen: description, project › task (search, create on the fly),
  tags, billable flag; Start / Pause·Resume / Stop. A pause writes nothing: it is the gap
  between the entries of one session, and it may last overnight.
- **Days are exact:** an entry never crosses local midnight. A running entry is cut at
  00:00 (also after sleep or a restart), a manual entry across midnight is split.
- **Time entries** by day with day totals: continue, edit, duplicate, delete, add
  manually (start/end or duration); optional "prevent overlapping entries".
- **Mini timer:** frameless, always on top, draggable, translucent until hovered;
  shows the session time, pause/resume and stop.
- **Tray:** icon shows running / paused / stopped; menu with pause, stop, mini timer,
  open, quit. Close (✕) hides to the tray; one instance only; start with Windows.
- **Dashboard:** recent tasks (quick start), last 7 days, activity heatmap, this week,
  time by project.
- **Projects, tasks, clients, tags:** colors, billable rates, time estimates with
  progress, archive, search and sort.
- **Summary report:** period presets, filters (projects, tasks, clients, tags,
  billable), two grouping levels (project, task, client, tag, billable, description, day),
  bar and pie charts, rounding; export to **CSV and XLSX**.
- **Settings:** language, date/time/duration/number/currency formats, first day of the
  week, time zone, dark/light theme, mini timer, backup and restore of the database.
- **Languages:** English, Russian, Simplified Chinese, Spanish
  ([add yours](docs/translating.md)).

## Install

Download `TimeTracker-<version>-setup.exe` from the release artifacts and run it. It
installs for the current user only (no administrator rights) into
`%LOCALAPPDATA%\Programs\TimeTracker` and adds a Start menu shortcut. Uninstall it from
Settings › Apps; your data is kept. A portable zip with just `tt-app.exe` is published
too.

The executable is not signed yet, so SmartScreen may warn on the first start.

## Your data

| What | Where |
|---|---|
| Database (SQLite, WAL) | `%APPDATA%\TimeTracker\timetracker.db` |
| Logs (no descriptions or names) | `%LOCALAPPDATA%\TimeTracker\logs` |

Settings › Data makes a backup copy of the database and restores one.

Command-line flags: `--minimized` starts in the tray (used by autostart); `--db <file>`
opens another database file instead (for trying things out; autostart is left alone).

## Build, test, run

Requirements: Windows 10/11, Rust 1.95 (MSVC toolchain, pinned in
`rust-toolchain.toml`).

```powershell
cargo run -p tt-app                     # debug build
cargo build --release -p tt-app         # target\release\tt-app.exe
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses               # needs cargo-deny
```

Demo data: `cargo run -p tt-app --example seed -- demo.db [--running]`, then
`cargo run -p tt-app -- --db demo.db`.

Installer (NSIS 3 with `makensis` on `PATH`):

```powershell
cargo build --release -p tt-app
makensis /DVERSION=0.1.0 crates\tt-app\installer\timetracker.nsi   # target\installer\
```

CI builds the installer and the portable zip for tags `v*`.

Workspace: `tt-domain` (rules, no I/O) → `tt-storage` (SQLite) → `tt-services`
(use cases, export, backup, updates) → `tt-app` (Slint UI, tray, platform code).

## Updates

The update check (signed manifest, verified installer) is built in but **off**: it is
compiled in only when both build-time variables are set, otherwise the Updates section
of the settings is hidden.

```powershell
$env:TT_UPDATE_URL = "https://example.com/timetracker/manifest.json"   # https only
$env:TT_UPDATE_PUBKEY = "<ed25519 public key, 64 hex digits>"
cargo build --release -p tt-app
```

In CI, set the repository variables `TT_UPDATE_URL` and `TT_UPDATE_PUBKEY`.

## License

Not chosen yet — all rights reserved for now.

The UI is built with [Slint](https://slint.dev) under the Slint Royalty-free License
2.0 (attribution in Settings › About). Third-party crate licenses are checked with
`cargo deny` (`deny.toml`).
