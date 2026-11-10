# Manual QA before a release

What tests cannot see. Tick each box; note anything odd with a screenshot.

To keep your real data out of it, start the app on a test database:
`tt-app.exe --db %TEMP%\tt-qa.db`. With `--db` the app does not touch "Start with Windows",
so check that one item without `--db`.

## Look

- [ ] **Light and Dark theme** (Settings › Theme). Check every page: Time, Dashboard,
  Projects, a project, Clients, Tags, Reports, Settings, and the mini timer.
  - Text readable, nothing white on white, focus rings and selections visible.
- [ ] **Languages** (Settings › Language): en, ru, es, zh.
  - No text cut off or overflowing buttons, especially in es (long words) and ru.
  - zh: characters render (no boxes), line height looks right.
- [ ] **zh input with an IME.** Type a description and a project name with the Microsoft
  Pinyin IME: the candidate window follows the cursor, the text lands in the field.
- [ ] **150 % scaling** (Windows Settings › Display › Scale). Text sharp, no clipped
  controls, the mini timer sized right.
- [ ] **Two monitors with different scaling.** Move the main window and the mini timer
  between them: both rescale, the mini timer stays on top.

## Tray and windows

- [ ] Close button hides to the tray (when that setting is on); the tray icon reopens it.
- [ ] **Restart Explorer** (Task Manager › Windows Explorer › Restart): the tray icon comes
  back and still works.
- [ ] Mini timer: on top of other windows, 70 % see-through until the mouse is over it,
  start / pause / stop work from it, it survives hiding and showing the main window.
- [ ] Timer runs across a pause left overnight: the next day the time is split per day.

## Installer (`TimeTracker-<version>-setup.exe`)

- [ ] Click through every page; texts readable, the version is right.
- [ ] Installs per user without an admin prompt, into `%LOCALAPPDATA%\Programs\TimeTracker`.
- [ ] Start-menu shortcut works; "Start with Windows" (Settings) starts it after sign-in.
- [ ] **Upgrade over a running copy:** the installer asks to close it, then replaces it;
  the data is still there.
- [ ] Uninstall removes the program and the shortcut and keeps the data in
  `%APPDATA%\TimeTracker`.

## Data

- [ ] Back up, add an entry, restore: the entry is gone, `timetracker.db.before-restore`
  exists next to the database and holds the entry.
- [ ] Export CSV and XLSX; the XLSX opens in Excel with localized sheet names.

## CI

- [ ] Push a `v*` tag: the `installer` job builds and uploads the installer and the
  portable zip as workflow artifacts.
