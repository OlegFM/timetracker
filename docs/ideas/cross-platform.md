# Idea: macOS and Linux builds

Status: Linux is in 0.2.0, macOS in 0.3.0 (ad-hoc signed, not notarized). Notarization is planned
after donations. Design:
[../superpowers/specs/2026-11-11-cross-platform-design.md](../superpowers/specs/2026-11-11-cross-platform-design.md).
Recorded 2026-11-10; the notes below are the original analysis.

## Where we stand

- **Already portable:** `tt-domain`, `tt-storage`, `tt-services` (timer, reports, export,
  backup, updates) use no Windows API. Slint and winit run on all three systems.
- **Data and log folders** come from `directories::BaseDirs`, so other systems get their
  native paths.
- **Windows-only code** lives in `crates/tt-app/src/platform/`, plus the installer and CI.

## What has to change

| Module | Today | macOS / Linux |
|---|---|---|
| `autostart.rs` | registry `Run` value (`windows-registry`) | `auto-launch` (already a dependency) does LaunchAgent / `.desktop`; drop the registry "same exe" check |
| `single_instance.rs` | Win32 named mutex | lock file or local socket |
| `fatal.rs` | Win32 `MessageBox` | a cross-platform dialog (e.g. `rfd`) or log only |
| `window_ext.rs` | icon from the exe resource; always-on-top already behind `cfg(target_os)` | icon from PNG; always-on-top via winit |
| `tray.rs` | `tray-icon` | works on macOS; on Linux needs GTK + libappindicator |
| Packaging, CI | NSIS, `windows-latest` | macOS `.dmg` + notarization; Linux AppImage / `.deb` / Flatpak; CI matrix |

## Known hard parts

- **Mini timer on Linux Wayland.** An app cannot keep its own window always on top
  there. This is a protocol limit, not a bug of ours. X11 works.
- **Linux tray.** GNOME has no tray by default; it needs the AppIndicator extension.
- **macOS.** The tray is a menu-bar item, and "close hides to tray" should follow Mac habits.
  - Without notarization the app does not open with a double click.
  - Notarization needs an Apple Developer account (USD 99 a year).
- **Testing.** It needs a real machine or VM per system, because the tests do not cover the UI.

## Rough size (estimate)

- **macOS:** a few days, mostly packaging and notarization.
- **Linux:** easy to build, harder to make good, because the tray and the mini timer depend
  on the desktop environment.
