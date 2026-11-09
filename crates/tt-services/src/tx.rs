use rusqlite::Connection;
use tt_storage::{Db, StorageError};

use crate::error::{Result, ServiceError};

/// Runs `f` in one `Db::transaction`; any error (storage or service) rolls back.
pub(crate) fn in_tx<T>(db: &mut Db, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let mut failure = None;
    let out = db.transaction(|tx| {
        f(tx).map_err(|e| match e {
            ServiceError::Storage(e) => e,
            other => {
                failure = Some(other);
                StorageError::Corrupt("aborted by service error".into())
            }
        })
    });
    out.map_err(|e| failure.unwrap_or(ServiceError::Storage(e)))
}
