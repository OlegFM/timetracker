//! Database backup (`VACUUM INTO`) and validated restore.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use tt_domain::{Millis, Tz};
use tt_storage::{Db, SCHEMA_VERSION, StorageError};

use crate::error::{Result, ServiceError};
use crate::timer::pause_running_in;

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ServiceError::Io(e)),
    }
}

/// Writes a consistent copy of the database to `dest`, replacing an existing file.
///
/// In the copy a running timer is paused at `now` (the entry closed and split at local
/// midnights of `tz` as the live timer would), so that restoring it later does not count
/// the time between the backup and the restore. The live database is not changed.
pub fn backup_to(db: &Db, dest: &Path, now: Millis, tz: Tz) -> Result<()> {
    let tmp = sibling(dest, ".tmp");
    remove_if_exists(&tmp)?;
    let res = db
        .conn()
        .execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])
        .map_err(sqlite)
        .and_then(|_| pause_copy(&tmp, now, tz))
        .and_then(|_| std::fs::rename(&tmp, dest).map_err(ServiceError::Io));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

fn sqlite(e: rusqlite::Error) -> ServiceError {
    ServiceError::Storage(StorageError::from(e))
}

/// Pauses the running session of the database at `path` (closed again on return).
fn pause_copy(path: &Path, now: Millis, tz: Tz) -> Result<()> {
    let mut conn = Connection::open(path).map_err(sqlite)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(sqlite)?;
    let tx = conn.transaction().map_err(sqlite)?;
    pause_running_in(&tx, now, tz)?;
    tx.commit().map_err(sqlite)
}

fn validate(src: &Path) -> std::result::Result<(), rusqlite::Error> {
    let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    conn.prepare("SELECT 1 FROM time_entries LIMIT 0")?;
    Ok(())
}

/// Replaces `live_path` with the backup at `src` after validating it. The replaced file
/// is kept as `<live>.before-restore` (an older one is overwritten).
/// The caller must have closed its `Db` and re-opens it afterwards.
pub fn restore_from(src: &Path, live_path: &Path) -> Result<()> {
    validate(src).map_err(|_| ServiceError::Invalid("backup"))?;
    let tmp = sibling(live_path, ".restore-tmp");
    std::fs::copy(src, &tmp).map_err(ServiceError::Io)?;
    let res = (|| {
        if live_path.exists() {
            std::fs::copy(live_path, sibling(live_path, ".before-restore"))
                .map_err(ServiceError::Io)?;
        }
        remove_if_exists(&sibling(live_path, "-wal"))?;
        remove_if_exists(&sibling(live_path, "-shm"))?;
        std::fs::rename(&tmp, live_path).map_err(ServiceError::Io)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(db: &Db) -> i64 {
        db.conn()
            .query_row("SELECT COUNT(*) FROM clients", [], |r| r.get(0))
            .unwrap()
    }

    fn add_client(db: &Db) {
        db.conn()
            .execute_batch(
                "INSERT INTO clients (id, name, created_at, updated_at) VALUES ('c1', 'x', 0, 0)",
            )
            .unwrap();
    }

    #[test]
    fn backup_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.db");
        let bak = dir.path().join("bak.db");
        let db = Db::open(&live).unwrap();
        let before = count(&db);
        backup_to(&db, &bak, 0, tt_domain::Tz::UTC).unwrap();
        backup_to(&db, &bak, 0, tt_domain::Tz::UTC).unwrap(); // overwrite is fine
        add_client(&db);
        assert_eq!(count(&db), before + 1);
        drop(db);
        restore_from(&bak, &live).unwrap();
        let db = Db::open(&live).unwrap();
        assert_eq!(count(&db), before);
    }

    #[test]
    fn backup_while_running_pauses_the_copy_at_the_backup_instant() {
        use crate::clock::FixedClock;
        use crate::timer::TimerService;
        use chrono::TimeZone;
        use tt_domain::{EntryTemplate, SessionState, Tz};
        use tt_storage::entries;

        const MSK: Tz = Tz::Europe__Moscow;
        let at = |d: u32, h: u32| {
            MSK.with_ymd_and_hms(2026, 11, d, h, 0, 0)
                .unwrap()
                .timestamp_millis()
        };
        let dir = tempfile::tempdir().unwrap();
        let (live, bak) = (dir.path().join("live.db"), dir.path().join("bak.db"));
        let mut db = Db::open(&live).unwrap();
        let svc = TimerService::new(FixedClock::new(at(9, 22)));
        let t = EntryTemplate {
            description: "a".into(),
            ..EntryTemplate::default()
        };
        svc.start(&mut db, t, MSK).unwrap();

        // Backup at T = 01:00 the next day, across a midnight without a rollover tick.
        let backup_at = at(10, 1);
        svc.clock().set(backup_at);
        backup_to(&db, &bak, backup_at, MSK).unwrap();
        // The live database keeps running.
        let running = entries::running_entry(db.conn()).unwrap().unwrap();
        assert_eq!(running.start, at(9, 22));
        drop(db);

        // Restored three days later: nothing was tracked after T.
        restore_from(&bak, &live).unwrap();
        let mut db = Db::open(&live).unwrap();
        svc.clock().set(at(13, 1));
        let st = svc.status(&db, MSK).unwrap();
        assert_eq!(st.session.as_ref().unwrap().state, SessionState::Paused);
        let all = entries::entries_in_range(db.conn(), i64::MIN, i64::MAX).unwrap();
        let spans: Vec<_> = all.iter().map(|e| (e.start, e.end)).collect();
        assert_eq!(
            spans,
            [(at(9, 22), Some(at(10, 0))), (at(10, 0), Some(backup_at))]
        );
        assert_eq!(st.session_ms, 3 * 3_600_000);

        let cur = svc.resume(&mut db, MSK).unwrap().current.unwrap();
        assert_eq!(
            (cur.start, cur.end, cur.description.as_str()),
            (at(13, 1), None, "a")
        );
    }

    #[test]
    fn restore_keeps_the_replaced_database_as_a_safety_copy() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("timetracker.db");
        let bak = dir.path().join("bak.db");
        let safety = dir.path().join("timetracker.db.before-restore");
        std::fs::write(&safety, b"older safety copy").unwrap();
        let db = Db::open(&live).unwrap();
        backup_to(&db, &bak, 0, tt_domain::Tz::UTC).unwrap();
        add_client(&db);
        drop(db);

        restore_from(&bak, &live).unwrap();
        assert_eq!(count(&Db::open(&live).unwrap()), 0);
        // The data from before the restore is still there, the older copy replaced.
        assert_eq!(count(&Db::open(&safety).unwrap()), 1);
    }

    #[test]
    fn restore_rejects_non_sqlite_file() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.db");
        let bad = dir.path().join("bad.db");
        std::fs::write(&bad, b"definitely not sqlite, just some text padding").unwrap();
        std::fs::write(&live, b"live").unwrap();
        let err = restore_from(&bad, &live).unwrap_err();
        assert!(matches!(err, ServiceError::Invalid("backup")));
        assert_eq!(std::fs::read(&live).unwrap(), b"live");
    }

    #[test]
    fn restore_rejects_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live.db");
        let newer = dir.path().join("newer.db");
        let db = Db::open(&newer).unwrap();
        db.conn()
            .execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION + 1))
            .unwrap();
        drop(db);
        std::fs::write(&live, b"live").unwrap();
        let err = restore_from(&newer, &live).unwrap_err();
        assert!(matches!(err, ServiceError::Invalid("backup")));
        assert_eq!(std::fs::read(&live).unwrap(), b"live");
    }
}
