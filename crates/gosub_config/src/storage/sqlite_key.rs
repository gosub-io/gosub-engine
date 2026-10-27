//! Keys for encrypted SQLite databases.
//!
//! Encryption is SQLCipher's, enabled with the `sqlcipher` feature. The key is a raw 256-bit
//! key: the caller derives it (the browser profile does), so SQLCipher's own passphrase
//! derivation is skipped. Without the feature, SQLite ignores the key pragma; [`SqliteKey::apply`]
//! checks for that and fails rather than let a store write plaintext it was told to encrypt.

use cow_utils::CowUtils;
use rusqlite::{ffi, Connection, OptionalExtension};
use std::fmt;
use std::path::{Path, PathBuf};
use zeroize::{Zeroize, Zeroizing};

/// A raw 256-bit key for one SQLite database. Wiped from memory on drop.
#[derive(Clone)]
pub struct SqliteKey([u8; 32]);

impl SqliteKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// `"x'<64 hex digits>'"`, the form SQLCipher takes as a raw key.
    fn pragma_value(&self) -> Zeroizing<String> {
        let mut s = Zeroizing::new(String::with_capacity(68));
        s.push_str("\"x'");
        for b in self.0 {
            s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
            s.push(char::from_digit(u32::from(b & 0xf), 16).unwrap_or('0'));
        }
        s.push_str("'\"");
        s
    }

    /// Unlock a freshly opened connection. Must run before any other statement on it. Fails
    /// when this SQLite has no SQLCipher, or when the file is plaintext or encrypted with a
    /// different key, so a wrong key shows up here and not on first use.
    pub fn apply(&self, conn: &Connection) -> rusqlite::Result<()> {
        let sql = Zeroizing::new(format!("PRAGMA key = {};", self.pragma_value().as_str()));
        conn.execute_batch(&sql)?;
        ensure_sqlcipher(conn)?;
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
    }

    /// Encrypt a plaintext database in place with this key. The file is written to a
    /// sibling first and renamed over the original, so a failure leaves the plaintext
    /// database as it was. Any WAL is folded in first and removed afterwards. Nothing else
    /// may have the database open.
    pub fn encrypt_file(&self, path: &Path) -> rusqlite::Result<()> {
        let tmp = sibling(path, "encrypting");
        let _ = std::fs::remove_file(&tmp);
        {
            let conn = Connection::open(path)?;
            ensure_sqlcipher(&conn)?;
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
            let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let sql = Zeroizing::new(format!(
                "ATTACH DATABASE '{}' AS encrypted KEY {};
                 SELECT sqlcipher_export('encrypted');
                 PRAGMA encrypted.user_version = {version};
                 DETACH DATABASE encrypted;",
                tmp.to_string_lossy().cow_replace('\'', "''"),
                self.pragma_value().as_str(),
            ));
            if let Err(e) = conn.execute_batch(&sql) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        }
        std::fs::rename(&tmp, path).map_err(io_error)?;
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(with_suffix(path, suffix));
        }
        Ok(())
    }
}

impl Drop for SqliteKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SqliteKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SqliteKey(..)")
    }
}

/// Whether a database file is plaintext SQLite, judged by its header. A missing or empty
/// file counts as not plaintext: there is nothing to encrypt.
pub fn is_plaintext_database(path: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut header = [0u8; 16];
    match std::fs::File::open(path) {
        Ok(mut f) => match f.read_exact(&mut header) {
            Ok(()) => Ok(&header == b"SQLite format 3\0"),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(e) => Err(e),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn ensure_sqlcipher(conn: &Connection) -> rusqlite::Result<()> {
    let version: Option<String> = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).optional()?;
    match version {
        Some(_) => Ok(()),
        None => Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_MISUSE),
            Some("encrypted database requested, but SQLite was built without SQLCipher".into()),
        )),
    }
}

fn io_error(e: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_IOERR), Some(e.to_string()))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn sibling(path: &Path, tag: &str) -> PathBuf {
    with_suffix(path, &format!(".{tag}"))
}

#[cfg(all(test, feature = "sqlcipher"))]
mod tests {
    use super::*;

    fn key(n: u8) -> SqliteKey {
        SqliteKey::from_bytes([n; 32])
    }

    #[test]
    fn opens_only_with_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        {
            let conn = Connection::open(&path).unwrap();
            key(1).apply(&conn).unwrap();
            conn.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('secret');")
                .unwrap();
        }
        assert!(!is_plaintext_database(&path).unwrap());
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(6).any(|w| w == b"secret"));

        let plain = Connection::open(&path).unwrap();
        assert!(plain
            .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .is_err());
        let wrong = Connection::open(&path).unwrap();
        assert!(key(2).apply(&wrong).is_err());

        let right = Connection::open(&path).unwrap();
        key(1).apply(&right).unwrap();
        let v: String = right.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "secret");
    }

    #[test]
    fn plaintext_file_refuses_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE t(v);")
            .unwrap();
        let conn = Connection::open(&path).unwrap();
        assert!(key(1).apply(&conn).is_err());
    }

    #[test]
    fn encrypts_a_plaintext_file_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.execute_batch("CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('kept'); PRAGMA user_version = 7;")
                .unwrap();
        }
        assert!(is_plaintext_database(&path).unwrap());
        key(3).encrypt_file(&path).unwrap();
        assert!(!is_plaintext_database(&path).unwrap());
        assert!(!with_suffix(&path, "-wal").exists());

        let conn = Connection::open(&path).unwrap();
        key(3).apply(&conn).unwrap();
        let v: String = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!((v.as_str(), version), ("kept", 7));
    }
}

#[cfg(all(test, not(feature = "sqlcipher")))]
mod tests {
    use super::*;

    #[test]
    fn a_key_without_sqlcipher_is_refused() {
        let conn = Connection::open_in_memory().unwrap();
        let err = SqliteKey::from_bytes([1; 32]).apply(&conn).unwrap_err();
        assert!(err.to_string().contains("without SQLCipher"), "{err}");
    }
}
