# Development

## Build, test, run

Requirements: Rust 1.95 (pinned in `rust-toolchain.toml`) on Windows 10/11 (MSVC
toolchain), on Linux (Ubuntu 22.04 / 24.04, see [Linux](#linux) below) or on macOS 11+ (see
[macOS](#macos) below).

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
Platform-specific code (Windows, Linux, macOS) lives in `crates/tt-app/src/platform/`.

Translations: [translating.md](translating.md). Manual checks before a release:
[qa-checklist.md](qa-checklist.md).

## Command-line flags

- **`--minimized`** starts in the tray or menu bar (used by autostart). On Linux it waits up to 10 s
  for a tray host at login, and shows the window minimized if there is none.
- **`--db <file>`** opens another database file instead, for trying things out.
  - Nothing then writes the autostart entry (the registry on Windows, the `.desktop` file on
    Linux).
  - The "Start with Windows" / "Start when you sign in" switch changes only the setting in
    that file.

## Installer

NSIS 3 (`makensis` on `PATH`, or `C:\Program Files (x86)\NSIS\makensis.exe`):

```powershell
cargo build --release -p tt-app
makensis /DVERSION=0.1.0 crates\tt-app\installer\timetracker.nsi   # target\installer\
```

**CI on a tag.** For tags `v*`, CI builds the installer and the portable zip (and the Linux
packages, see [Linux](#linux)) and keeps them as workflow artifacts of that run.

**Publishing.** The GitHub Release is made by hand from those artifacts:

```powershell
gh run download <run-id> -D <empty-folder>
gh release create v0.1.0 --verify-tag --notes-file <notes.md> <files>
```

## Linux

Target: Ubuntu 22.04 and 24.04 with GNOME, x86_64. CI builds on `ubuntu-22.04`.

**Build packages (Ubuntu):**

```bash
sudo apt install build-essential pkg-config libfontconfig1-dev libxkbcommon-dev
```

To run the app you also need `libxkbcommon-x11-0` (winit loads it at runtime; the `.deb`
depends on it, and Ubuntu Desktop has it). Then the same commands as on Windows:
`cargo run -p tt-app`, `cargo test --workspace`, and so on. The update check is not built
on Linux.

**Linux code builds only on Linux.** `rusqlite` with `bundled` compiles C code, so the
Linux part cannot be built on Windows. On a Windows machine, use WSL2 with Ubuntu. Install
the tools once (needs a network):

```bash
sudo apt-get update && sudo apt-get install -y build-essential pkg-config libfontconfig1-dev libxkbcommon-dev x11-utils
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.95 --profile minimal -c rustfmt,clippy
```

Build in a directory outside `/mnt/<drive>`, so it does not mix with the Windows `target`
and avoids the slow 9p file system:

```bash
wsl -d Ubuntu -- bash -lc 'cd /mnt/e/Work/TimeTracker && export CARGO_TARGET_DIR=$HOME/tt-target && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace'
```

WSLg can show the app window, but it has no tray host, so the tray cannot be checked there.
Use a virtual machine for that ([qa-checklist.md](qa-checklist.md)).

**Packages.** Files are in `crates/tt-app/installer/linux/`
(`timetracker.desktop`, `build-appimage.sh`, `deb-version.sh`). They need `cargo-deb` 3.8.0
and `appimagetool` 1.9.1 (versions and checksum: [refs.md](refs.md)).

```bash
cargo build --release -p tt-app
cargo deb -p tt-app --no-build --deb-version <deb-version> -o target/installer/
crates/tt-app/installer/linux/build-appimage.sh <version> target/release/tt-app
```

- The `.deb` is `target/installer/timetracker_<deb-version>_amd64.deb`. Release candidates
  use `~rc.N` in the `.deb` version (`deb-version.sh` produces it). GitHub may show `~` as
  `.` in asset names.
- The AppImage is `target/installer/TimeTracker-<version>-x86_64.AppImage`.

**Pre-release.** A tag `vX.Y.Z-rc.N` makes CI build all files and publish a GitHub
Pre-release by itself:

```bash
git tag v0.2.0-rc.1 && git push origin v0.2.0-rc.1
```

A final release (tag `vX.Y.Z`) is still published by hand from the CI artifacts, as above.

## macOS

Target: macOS 11 or newer, Apple Silicon and Intel (one universal binary). Build tools:
Xcode command line tools and Rust 1.95; the commands are the same as on Windows.

**macOS code builds only on macOS.** `rusqlite` with `bundled` compiles C code and needs the
Apple SDK, so it cannot be built on Windows. The macOS part is checked by the `check` job on
`macos-15` (the pure logic is also tested on Windows). Behaviour on a live Mac (Dock, menu bar,
reopening, autostart) is checked by hand with the macOS section of
[qa-checklist.md](qa-checklist.md).

**Package.** The job `macos-package` (`macos-15`) builds `aarch64-apple-darwin` and
`x86_64-apple-darwin` with `MACOSX_DEPLOYMENT_TARGET=11.0` and runs
`crates/tt-app/installer/macos/build-dmg.sh`:

```bash
crates/tt-app/installer/macos/build-dmg.sh <version> <arm64-binary> <x86_64-binary>
```

- The script joins the two binaries with `lipo` into one universal binary, makes
  `TimeTracker.app` (`Info.plist`, `app.icns`), signs it ad hoc
  (`codesign -s -`, no Apple certificate), and `hdiutil` packs
  `target/installer/TimeTracker-<version>-macos-universal.dmg`.
- A tag `vX.Y.Z-rc.N` publishes the `.dmg` in the Pre-release with the other files.
- CI also runs on pushes to `feature/**` branches.
- The icon `crates/tt-app/assets/app.icns` is generated by `gen_icon.py`.
- The update check is not built on macOS: CI gives that build no `TT_UPDATE_URL` and
  `TT_UPDATE_PUBKEY`.

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
