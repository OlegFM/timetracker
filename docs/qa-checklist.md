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

## Windows startup order

- [ ] At startup the mini timer appears before the main window, and focus ends up in the
  right place (the main window, not stuck behind the mini timer). Check a normal start and a
  `--minimized` one.

## Linux (Ubuntu 22.04 and 24.04, virtual machine)

Run it on both versions, once in a Wayland session and once in an X11 session.

- [ ] **Install from the AppImage:** `chmod +x`, double-click; the window opens.
- [ ] **Install from the `.deb`:** `sudo apt install ./timetracker_*_amd64.deb`; TimeTracker
  is in the applications menu and starts from it; the icon is right.
- [ ] **Timer:** start, pause, resume, stop; the time is right.
- [ ] **Mini timer:** stays on top of other windows; its position is the same after a
  restart.
- [ ] **Tray:** left click shows and hides the window; right click opens the menu; every
  menu item works; the first (disabled) item shows the current task.
- [ ] **Close to tray:** the close button hides the window, the icon reopens it.
- [ ] **No tray:** turn off the AppIndicator extension, then start the app. The icon is gone, the "close to
  tray" switch is disabled, the close button quits, and a running timer is still running
  after the next start.
- [ ] Turn off the extension while the app runs, then click ✕: the app quits (it does not
  hide).
- [ ] **Autostart:** "Start when you sign in" on and off; after signing in the app starts
  in the tray (with the extension off, it shows the window minimized after ~10 s).
- [ ] Turning it off in GNOME "Startup Applications" / Tweaks turns the setting off in the
  app.
- [ ] **Second launch** brings the running window forward instead of opening another one.
- [ ] **Backup and restore** (as in Data below).
- [ ] **Export CSV and XLSX** open in LibreOffice or similar.
- [ ] **Four languages** (en, ru, es, zh): texts fit, zh renders.
- [ ] **Wayland and X11 sessions:** the app starts in both (it runs on X11/XWayland).
- [ ] **Windows:** a short run of the sections "Tray and windows" and "Installer" above.

## macOS (11 or newer, a real Mac or a virtual machine)

Install the `.dmg` from the pre-release. Run it on Apple Silicon and on Intel if you have both.
Back up the data first, or use `open -n /Applications/TimeTracker.app --args --db /tmp/tt-qa.db`
(with `--db` the app does not touch autostart, so check that one item without it).

- [ ] **Install:** open the `.dmg`, drag TimeTracker to Applications, first start through the
  Gatekeeper steps from the README (Open Anyway on macOS 15+, right-click → Open on 11-14).
- [ ] **Icon** in the Dock and in Applications (Launchpad) looks right.
- [ ] **Timer:** start, pause, resume, stop; the time is right; a pause left overnight and a
  running entry across midnight split per day.
- [ ] **Mini timer:** on top of other windows, draggable, same position after a restart.
- [ ] **Mini timer without the main window:** hide the main window; the mini timer stays
  visible, and there is no Dock icon and no entry in `Cmd+Tab`.
- [ ] **Dock icon follows the main window:** it appears when the window is opened (menu bar
  "Open window", reopening), disappears when the window is closed with the close button, and
  the app is in `Cmd+Tab` only while the window is open.
- [ ] **Menu bar icon:** its colour follows the timer state (running, paused, stopped); every
  menu item works; a left click opens the menu and the window does not flash, show or hide.
- [ ] **Close to the menu bar** on and off (Settings): the label says "menu bar"; on, the
  close button hides the window; off, it quits.
- [ ] **`Cmd+Q` with the timer running:** after the next start the timer is still running.
- [ ] **Autostart** on and off; after signing in the app sits only in the menu bar, with no
  Dock icon (a brief flash is acceptable).
- [ ] **Autostart from the `.dmg`:** run the app from the mounted `.dmg` (or from Downloads),
  turn autostart on: no `TimeTracker.plist` appears in `~/Library/LaunchAgents`, and the log
  has a warning.
- [ ] **Reopen while hidden:** with the app only in the menu bar (no Dock icon), open it
  again from Finder, from Launchpad and from Spotlight: the main window shows each time.
- [ ] **Second launch from Terminal:** `open -n /Applications/TimeTracker.app` brings up the
  window of the first copy and does not start another.
- [ ] **Backup and restore**, **export CSV and XLSX** (as in Data below).
- [ ] **Four languages** (en, ru, es, zh): texts fit, zh renders with a Chinese font.

## Data

- [ ] Back up, add an entry, restore: the entry is gone, `timetracker.db.before-restore`
  exists next to the database and holds the entry.
- [ ] Export CSV and XLSX; the XLSX opens in Excel with localized sheet names.

## CI

- [ ] Push a `v*` tag: the `installer` job builds and uploads the installer and the
  portable zip as workflow artifacts.
- [ ] Push a `vX.Y.Z-rc.N` tag: a GitHub Pre-release appears with the installer, the zip,
  the AppImage, the `.deb` and the macOS `.dmg`.
