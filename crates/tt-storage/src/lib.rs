//! SQLite storage: schema, migrations and repository functions over `&Connection`.

pub mod catalog;
pub mod db;
pub mod entries;
pub mod settings;

pub use db::{Db, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Unparseable value read back from the database (bad uuid / enum text).
    #[error("corrupt data: {0}")]
    Corrupt(String),
    #[error("migration: {0}")]
    Migration(String),
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            // Raised by the column readers below for values that fail to parse.
            rusqlite::Error::FromSqlConversionFailure(_, _, ref inner) => {
                StorageError::Corrupt(inner.to_string())
            }
            other => StorageError::Sqlite(other),
        }
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Column readers / writers shared by the repositories.
pub(crate) mod cols {
    use rusqlite::{Row, types::Type};
    use tt_domain::Id;

    fn parse(idx: usize, s: &str) -> rusqlite::Result<Id> {
        Id::parse_str(s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                idx,
                Type::Text,
                format!("bad uuid {s:?}: {e}").into(),
            )
        })
    }

    pub fn id(row: &Row, idx: usize) -> rusqlite::Result<Id> {
        parse(idx, &row.get::<_, String>(idx)?)
    }

    pub fn opt_id(row: &Row, idx: usize) -> rusqlite::Result<Option<Id>> {
        row.get::<_, Option<String>>(idx)?
            .map(|s| parse(idx, &s))
            .transpose()
    }

    pub fn text(id: Id) -> String {
        id.hyphenated().to_string()
    }

    pub fn opt_text(id: Option<Id>) -> Option<String> {
        id.map(text)
    }
}
