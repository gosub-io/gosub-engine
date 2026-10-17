//! Encryption keys for the engine's SQLite stores. The key type and the file-level
//! helpers live in `gosub_config`, which opens the settings database the same way.

use r2d2_sqlite::rusqlite;
use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

pub use gosub_config::storage::{is_plaintext_database, SqliteKey};

/// A pool manager for `path` that unlocks every new connection with `key` before anything
/// else runs on it. No key leaves the database plaintext. The file is checked once here, on a
/// connection of its own: a pool retries a connection that fails its checkout until the pool
/// timeout runs out, so a wrong key, or an encrypted file opened without one, would otherwise
/// stall the open for minutes before reporting anything.
pub(crate) fn manager(path: &Path, key: Option<SqliteKey>) -> rusqlite::Result<SqliteConnectionManager> {
    let conn = rusqlite::Connection::open(path)?;
    let manager = SqliteConnectionManager::file(path);
    match key {
        Some(key) => {
            key.apply(&conn)?;
            Ok(manager.with_init(move |c| key.apply(c)))
        }
        None => {
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
            Ok(manager)
        }
    }
}
