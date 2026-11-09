//! Encryption keys for the engine's SQLite stores. The key type and the file-level
//! helpers live in `gosub_config`, which opens the settings database the same way.

use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};

pub use gosub_config::storage::{is_plaintext_database, SqliteKey};

/// A pool manager for `path` that unlocks every new connection with `key` before anything
/// else runs on it. No key leaves the database plaintext. The file is checked once here, on a
/// connection of its own: a pool retries a connection that fails its checkout until the pool
/// timeout runs out, so a wrong key, or an encrypted file opened without one, would otherwise
/// stall the open for minutes before reporting anything.
pub(crate) fn manager(path: &Path, key: Option<SqliteKey>) -> rusqlite::Result<SqliteConnectionManager> {
    let conn = Connection::open(path)?;
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

type InitFn = dyn Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync;

/// An r2d2 manager for a SQLite file, in place of `r2d2_sqlite`, which lags behind the
/// rusqlite releases that carry SQLite fixes. Every new connection opens `path` with the
/// flags and then runs the init.
pub struct SqliteConnectionManager {
    path: PathBuf,
    flags: OpenFlags,
    init: Option<Box<InitFn>>,
}

impl SqliteConnectionManager {
    pub fn file(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            flags: OpenFlags::default(),
            init: None,
        }
    }

    pub fn with_flags(self, flags: OpenFlags) -> Self {
        Self { flags, ..self }
    }

    pub fn with_init(self, init: impl Fn(&mut Connection) -> rusqlite::Result<()> + Send + Sync + 'static) -> Self {
        Self {
            init: Some(Box::new(init)),
            ..self
        }
    }
}

impl std::fmt::Debug for SqliteConnectionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteConnectionManager")
            .field("path", &self.path)
            .field("flags", &self.flags)
            .finish_non_exhaustive()
    }
}

impl r2d2::ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> rusqlite::Result<Connection> {
        let mut conn = Connection::open_with_flags(&self.path, self.flags)?;
        if let Some(init) = &self.init {
            init(&mut conn)?;
        }
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> rusqlite::Result<()> {
        conn.execute_batch("")
    }

    fn has_broken(&self, _: &mut Connection) -> bool {
        false
    }
}
