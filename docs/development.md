# Development

## Build, test, run

Requirements: Windows 10/11 and Rust 1.95 (MSVC toolchain, pinned in `rust-toolchain.toml`).
macOS and Linux builds are planned ([ideas/cross-platform.md](ideas/cross-platform.md)).

```powershell
cargo run -p tt-app                     # debug build
cargo build --release -p tt-app         # target\release\tt-app.exe
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo deny check licenses advisories    # needs cargo-deny
```

Demo data: `cargo run -p tt-app --example seed -- demo.db [--running]`, then
`cargo run -p tt-app -- --db demo.db`. Only one copy of the app runs per user session,
so quit an installed copy first.

Workspace: `tt-domain` (rules, no I/O) → `tt-storage` (SQLite) → `tt-services`
(use cases, export, backup, updates) → `tt-app` (Slint UI, tray, platform code).
Windows-specific code lives in `crates/tt-app/src/platform/`.

Translations: [translating.md](translating.md). Manual checks before a release:
[qa-checklist.md](qa-checklist.md).

## Command-line flags

- **`--minimized`** starts in the tray (used by autostart).
- **`--db <file>`** opens another database file instead, for trying things out.
  - Nothing then writes the autostart entry in the registry.
  - The "Start with Windows" switch changes only the setting in that file.

## Installer

NSIS 3 (`makensis` on `PATH`, or `C:\Program Files (x86)\NSIS\makensis.exe`):

```powershell
cargo build --release -p tt-app
makensis /DVERSION=0.1.0 crates\tt-app\installer\timetracker.nsi   # target\installer\
```

**CI on a tag.** For tags `v*`, CI builds the installer and the portable zip and keeps them
as workflow artifacts of that run.

**Publishing.** The GitHub Release is made by hand from those artifacts:

```powershell
gh run download <run-id> -D <empty-folder>
gh release create v0.1.0 --verify-tag --notes-file <notes.md> <files>
```

## Updates

The update check (signed manifest, verified installer) is built in but **off**. It is
compiled in only when both build-time variables are set; otherwise the Updates section of
the settings is hidden. Before turning it on, see [known-issues.md](known-issues.md).

```powershell
$env:TT_UPDATE_URL = "https://example.com/timetracker/manifest.json"   # https only
$env:TT_UPDATE_PUBKEY = "<ed25519 public key, 64 hex digits>"
cargo build --release -p tt-app
```

In CI, set the repository variables `TT_UPDATE_URL` and `TT_UPDATE_PUBKEY`.

## Data details

- **Database:** SQLite in WAL mode.
- **Times:** UTC milliseconds, half-open intervals.
- **Running entry:** the one with `end IS NULL`.
- **Days:** entries are split at local midnight.

**XLSX export.** The report export has two sheets.

- **Summary:** the running entry is counted up to now, and rounding is applied.
- **Entries:** closed entries only, with raw times and no rounding.

**Backup.** A timer running at the time of a backup is saved as paused at that moment, so
restoring the backup does not count the time in between.

**Restore.** Before a restore, the current database is kept next to it as
`timetracker.db.before-restore`.

- It replaces an older copy of that name.
- Changes still in `-wal` are folded into this one self-contained file.
- To undo a wrong restore, copy that file back while the app is closed, or restore it from
  Settings › Data.
