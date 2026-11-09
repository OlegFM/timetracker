//! Database backup (`VACUUM INTO`) and validated restore.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use tt_storage::{Db, SCHEMA_VERSION, StorageError};

use crate::error::{Result, ServiceError};

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
pub fn backup_to(db: &Db, dest: &Path) -> Result<()> {
    let tmp = sibling(dest, ".tmp");
    remove_if_exists(&tmp)?;
    let res = db
        .conn()
        .execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])
        .map_err(|e| ServiceError::Storage(StorageError::from(e)))
        .and_then(|_| std::fs::rename(&tmp, dest).map_err(ServiceError::Io));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
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

/// Replaces `live_path` with the backup at `src` after validating it.
/// The caller must have closed its `Db` and re-opens it afterwards.
pub fn restore_from(src: &Path, live_path: &Path) -> Result<()> {
    validate(src).map_err(|_| ServiceError::Invalid("backup"))?;
    let tmp = sibling(live_path, ".restore-tmp");
    std::fs::copy(src, &tmp).map_err(ServiceError::Io)?;
    let res = (|| {
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
        backup_to(&db, &bak).unwrap();
        backup_to(&db, &bak).unwrap(); // overwrite is fine
        add_client(&db);
        assert_eq!(count(&db), before + 1);
        drop(db);
        restore_from(&bak, &live).unwrap();
        let db = Db::open(&live).unwrap();
        assert_eq!(count(&db), before);
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
