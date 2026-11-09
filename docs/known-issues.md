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

- **Future-dated entries do not appear in the entries list.** An entry with a wrong date seems to vanish.
- **The two XLSX sheets give different totals.**
  - The Summary sheet is rounded and includes the running entry.
  - The Entries sheet is raw and skips running entries.
  - This is documented in the README, not resolved.
- **The safety copy can miss recent data after a crash.** `timetracker.db.before-restore` copies only the main file. After an unclean close, data still in `-wal` is missing from it. Fix: checkpoint before closing, or copy `-wal` too.
- **Run `cargo deny check advisories` before the first public release.** This matters for the network stack: ureq and rustls.
- **Manual QA before release.**
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
