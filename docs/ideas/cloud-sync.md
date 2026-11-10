# Idea: sync between devices through a cloud folder

Status: idea, after v0.1.0. Not designed yet. Recorded 2026-11-10.

## Why

The same person tracks time on more than one device, for example a laptop used inside one
client's corporate network and a desktop PC for another client. Clients, projects, tasks
and tags should be shared. Today each device has its own database.

No sync server: a cloud drive folder (OneDrive, Dropbox, Google Drive, ...) carries the data.

## What was decided

- **A setting picks the cloud folder** that holds the shared database.
- **Each running copy works on its own local database**, never on the shared file
  directly. The local database stays where it is now (`%APPDATA%\TimeTracker`).
- **Sync runs about once an hour and when the app closes.** It merges local changes into
  the shared database and takes the other devices' changes back.
- **Lock files guard the merge.**
  - Before merging, a copy puts `.dblock-<instance id>` into the folder.
  - About a minute later it checks again. If there are two lock files, the copy whose lock
    file was created first merges first.
- **Conflicts are rare.** Entries are bound to a device in practice, so the same record is
  hardly ever edited on two devices. When it happens, the later edit wins.
- **Entries carry the device** they were tracked on (a new column). A timer still running
  on another device shows as an entry of that device, with its mark. It is not a second
  running timer on this device.

## Points to settle in the design

These are notes from the discussion, not decisions.

- **Merge by record, not by file copy.** Records already have ids and `updated_at`.
- **Deletes need a marker** (tombstone). Without one, a record deleted on one device comes
  back from another.
- **Pull on start.** Take the shared changes before the user starts working.
- **Lock files are a hint, not a guarantee.** Cloud drives deliver files with a delay, so two
  devices can both believe they are alone.
  - This is safe if each device keeps all its own changes locally and offers them again at
    every merge. A lost merge, or a "conflicted copy" made by the cloud client, then heals on
    the next run.
- **Stale lock files.** A lock left by a crashed copy must expire, for example after a few
  minutes.
- **Long offline stretches.** The corporate network may block the cloud drive, so a laptop
  may sync only every few days. Local changes queue up until then.
- **One running timer.** Today the database allows one running entry (`one_running`
  index). With entries per device, this rule becomes "one per device".
- **Schema change.** The device column, tombstones and any sync bookkeeping need a
  migration, an ADR, and compatibility tests.
- **Backups and restore.** Restoring an old backup on one device must not wipe the shared
  data on the next sync.
