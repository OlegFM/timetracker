# Known issues and follow-ups (v0.1.0)

Found during the v1 reviews. None block v0.1.0. Nothing here is in the shipped code path unless noted.

## Must be done before updates are enabled

Updates are off in every build until `TT_UPDATE_*` is set at build time.

- **Sign the manifest version.** Today the signature covers only the installer bytes, so an older signed installer could be replayed under a higher manifest version (downgrade). Sign version and sha256 together.
- **Pre-release versions.** A current version like `0.2.0-beta.1` never compares as older, so it never updates.
- **Update path tidy-up.**
  - The installer goes to a fixed `%TEMP%` path that is never cleaned.
  - A second update check can start while one is still running.
  - `controllers/settings.rs` (~670 lines) should then be split: backup/restore and updates into their own controllers.

## Worth fixing in v0.1.x

- **The two XLSX sheets give different totals.**
  - The Summary sheet is rounded and includes the running entry.
  - The Entries sheet is raw and skips running entries.
  - This is documented in [development.md](development.md), not resolved.
- **Manual QA before release** ([qa-checklist.md](qa-checklist.md)).
  - Light theme, es/zh_CN screens and zh IME input.
  - 150 % DPI and multiple monitors.
  - Re-adding the tray icon after Explorer restarts.
  - Clicking through the NSIS installer pages.
  - The CI tag job.

## Polish

- **Reports.**
  - Pie chart top-8 under Day grouping shows the 8 earliest days.
  - With ~31 daily bars, the labels touch.
  - In ru, "Нет" is used for both None and No.
- **Export.**
  - The `names` identity in export relies on `ptr::eq`. It is correct today, but should take a level argument.
  - `vm/report.rs` is large; split it into filters/charts/table.
- **Installer.** The "polite close" step waits ~3 s, because WM_CLOSE hides the app to the tray.
- **Startup.** `main.rs` shows one untranslated startup error before the UI exists.
- **Tests.** Several are thin:
  - the XLSX workbook contents;
  - controllers;
  - the `--db` → no-autostart wiring.

## Linux

- **The mini timer shows in the Dock and in `Alt+Tab`.** It is a normal window to GNOME.
- **Slight blur on fractional scaling.** The app runs on X11/XWayland even in a Wayland
  session, and the compositor scales XWayland windows up.
- **A Wayland session without XWayland cannot run the app.** The app removes
  `WAYLAND_DISPLAY` at start so that the always-on-top mini timer and window positions work.
- **No tooltip on the tray icon in GNOME.** The current task is the first (disabled) item
  of the menu instead.
- **No tray host, no icon.** On GNOME without the AppIndicator extension the close button
  quits the app.
- **Removing the app leaves the autostart file.** Removing the .deb or deleting/moving the
  AppImage leaves `~/.config/autostart/TimeTracker.desktop` pointing at nothing until the app
  is started again. Turn autostart off before removing, or delete the file.

## Planned after v0.1.0

- **Sync between devices through a cloud folder.** See [ideas/cloud-sync.md](ideas/cloud-sync.md).
- **Microsoft Store (MSIX) release**, so users get no SmartScreen warning. See [ideas/msix-store.md](ideas/msix-store.md).
- **A macOS build (0.3.0).** Linux is in 0.2.0. See [ideas/cross-platform.md](ideas/cross-platform.md).
