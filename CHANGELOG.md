# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The project is licensed under AGPL-3.0-only (`LICENSE`, Cargo `license` fields, installer version info).

### Fixed

- Entries with a future date (for example a mistyped year) are listed again, at the top; before they were hidden from the entries list.
- The `timetracker.db.before-restore` safety copy keeps changes that were still in the `-wal` file after an unclean close, and is one self-contained file.

### Added

- CI checks dependencies for known vulnerabilities (`cargo deny check advisories`).

## [0.1.0] - 2026-11-09

First version: a standalone Windows time tracker whose behaviour is modelled on the core
of solidtime (not affiliated; the code is our own).

### Added

- Timer bar on every screen: description, project › task picker with search and
  create-on-the-fly, tag multi-select, billable flag, Start / Pause·Resume / Stop.
- Sessions with pause: a pause closes the current entry and writes nothing else;
  Resume continues the session with a new entry; a pause may last overnight.
- Entries never cross local midnight: a running entry is cut at 00:00 (also after sleep,
  a crash or a restart), manual entries across midnight are split; DST-safe day bounds.
- Time entries screen: entries by day with day totals; continue, edit, duplicate,
  delete; manual entries by start/end or duration; optional overlap prevention.
- Floating mini timer: frameless, always on top, draggable, translucent until hovered,
  remembers its position.
- Tray icon (running / paused / stopped) with menu; close to tray; single instance;
  start with Windows (`--minimized`).
- Dashboard: recent tasks with quick start, last 7 days, activity heatmap, this week,
  time by project.
- Projects (color, client, billable rate, estimate with progress, archive), project page
  with tasks, clients, tags; search, sort, archived filter, guarded delete.
- Billable rates: project rate or the default rate; money in minor units.
- Summary report: period presets, filters, two grouping levels, bar and pie charts,
  rounding per entry; export of the report and the entries to CSV (UTF-8 with BOM,
  formula-safe) and XLSX.
- Settings: language, date/time/duration/number/currency formats, week start, time zone,
  theme, mini timer opacity, backup and restore of the database, About with Slint
  attribution.
- Update check with signed manifest and verified installer, disabled unless
  `TT_UPDATE_URL` and `TT_UPDATE_PUBKEY` are set at build time.
- Translations: English, Russian, Simplified Chinese, Spanish.
- Per-user NSIS installer (no administrator rights, Start menu shortcut, uninstall keeps
  the data, asks before closing a running copy), portable zip, application icon (file,
  title bar, taskbar) and version resource.
- `--db <file>` flag to open another database file (it never writes the autostart
  entry); `seed` example `--running` option.
- Release profile tuned for size and memory; Slint software renderer only
  ([docs/perf.md](docs/perf.md)).
- A restore keeps the replaced database as `timetracker.db.before-restore`.

### Fixed

- Deleting a tag, project or task named by the paused (or running) timer no longer makes
  Resume, Stop or Start fail; the timer and the timer bar draft simply drop it.
- A backup taken while the timer runs stores it paused at the backup moment: restoring it
  later no longer counts the time in between.
- Duplicate is disabled while overlapping entries are not allowed (the copy always
  overlaps its original).
- An entry closed at or before its start (stopped at once, or the clock was moved back)
  is dropped instead of stored with zero length.
- Turning "Start with Windows" off removes the Run entry only when it starts this copy of
  the app.
- XLSX sheet names and the restore error box are translated; the updater uses HTTPS only
  (also after redirects) and a slow installer download is no longer cut after 10 s.
