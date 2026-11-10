# TimeTracker

**An offline time tracker. No account, no server, no subscription. Your data stays on
your computer.**

[![Release](https://img.shields.io/github/v/release/OlegFM/timetracker)](https://github.com/OlegFM/timetracker/releases/latest)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue)](LICENSE)
![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%26%20Linux%20soon-lightgrey)

English · [Русский](README.ru.md)

![Time entries in the dark theme, with the mini timer on top](docs/screenshots/hero.png)

Track time per client, project and task. Pause the timer and resume it later, even the
next day. At the end of the month, export a report with hours and money per client.
Everything runs locally in a small native app.

## Why TimeTracker

- **Works offline, with no account.** Nothing to sign up for, nothing sent anywhere. The
  data is one SQLite file on your disk. You can back it up, restore it and export it.
- **Light and native.** No browser engine inside.
  - It starts in about 0.2 s.
  - In the tray with the timer running it uses about 4–6 MB of memory and practically no
    CPU ([measurements](docs/perf.md)).
- **A mini timer on top of other windows.** It is translucent until you move the mouse over
  it, and has pause and stop right there.
- **Honest days.** A pause may last overnight. An entry never crosses midnight, so every
  day gets exactly its own time.
- **Billing built in.** Hourly rates per project, with a default rate. Reports show time
  and cost and export to CSV and XLSX.
- **Four languages:** English, Russian, Spanish, Simplified Chinese
  ([add yours](docs/translating.md)).

## For freelancers

You work for several clients and need to bill each one exactly.

1. **Set up once.** Add a client, a project with its hourly rate, and a few tasks.
2. **Track.** Start the timer from the timer bar, the tray, the mini timer or the
   dashboard's recent tasks, and pause it for a break.
   - Forgot to start it? Add the entry by hand afterwards.
3. **Bill.**
   - Open Reports and pick the month.
   - Filter by client and group by project and task.
   - The total cost is there; export it to XLSX for the invoice.

## Leaving a cloud tracker

| | TimeTracker | Typical cloud tracker |
|---|---|---|
| Account | not needed | required |
| Works offline | always | limited or not at all |
| Where your data is | a file on your computer | the vendor's servers |
| Price | free, open source | free tier, paid plans |
| Teams, shared workspaces | no, it is a personal tool | yes |
| Sync between devices | planned, through a cloud folder of your choice | yes |
| Mobile app | no | usually |

Its behaviour is modelled on the core of [solidtime](https://www.solidtime.io/). It is not
affiliated with solidtime, and the code is our own.

## A quick tour

| | |
|---|---|
| ![Dashboard](docs/screenshots/dashboard.png) | **Dashboard.** Recent tasks to start in one click, the last 7 days, an activity heatmap, this week, and time by project. |
| ![Reports in the light theme](docs/screenshots/reports.png) | **Reports** (here in the light theme). Period presets, filters, two grouping levels, bar and pie charts, rounding, and CSV and XLSX export. |
| ![Projects](docs/screenshots/projects.png) | **Projects, tasks, clients, tags.** Colors, hourly rates, time estimates with progress, and an archive. |
| ![Mini timer](docs/screenshots/mini-timer.png) | **Mini timer.** Always on top, draggable, translucent until the mouse is over it. |

The tray icon shows whether the timer is running, paused or stopped. Its menu can pause,
stop, show the mini timer or open the window. Closing the window hides it to the tray.

## Install

**Windows 10/11, 64-bit.** Download from the [latest release](https://github.com/OlegFM/timetracker/releases/latest):

- **`TimeTracker-<version>-setup.exe`** installs for the current user only, without
  administrator rights.
  - It adds a Start menu shortcut.
  - Uninstalling keeps your data.
- **`TimeTracker-<version>-portable.zip`** contains just `tt-app.exe`, with no installation.

The files are not signed yet, so Windows SmartScreen may warn on the first start. Choose
**More info** → **Run anyway**.

**macOS and Linux** builds are coming soon ([plan](docs/ideas/cross-platform.md)).

## Your data

| What | Where |
|---|---|
| Database | `%APPDATA%\TimeTracker\timetracker.db` |
| Logs (no descriptions or names) | `%LOCALAPPDATA%\TimeTracker\logs` |

**Settings › Data** makes a backup copy and restores one. Before a restore, the current
database is kept as `timetracker.db.before-restore`, so a wrong restore can be undone.

## Roadmap

- Sync between devices through a cloud folder (OneDrive, Dropbox, Google Drive…), with no
  server ([idea](docs/ideas/cloud-sync.md)).
- macOS and Linux builds ([idea](docs/ideas/cross-platform.md)).
- Microsoft Store release, signed by Microsoft ([idea](docs/ideas/msix-store.md)).

## Support the project

TimeTracker is free. If it saves you time, you can support its development:

- **In rubles** (cards of Russian banks, SBP): *CloudTips link coming soon*
- **In crypto:** *wallet addresses coming soon*

## Contributing

Bug reports and ideas are welcome in [Issues](https://github.com/OlegFM/timetracker/issues).
To build from source, run the tests or add a translation, see
[docs/development.md](docs/development.md).

Written in Rust with [Slint](https://slint.dev).

## License

[GNU AGPL-3.0-only](LICENSE).

The UI is built with [Slint](https://slint.dev) under the Slint Royalty-free License 2.0
(attribution in Settings › About). Third-party crate licenses are checked with
`cargo deny` (`deny.toml`).
