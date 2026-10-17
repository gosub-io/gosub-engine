//! Each SQLite store opened with a key writes a file that only that key opens.

#![cfg(all(feature = "sqlcipher", feature = "sqlite_places"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_engine::config_storage::SqliteStorageAdapter;
use gosub_engine::cookies::SqliteCookieStore;
use gosub_engine::places::{Places, SqlitePlaces};
use gosub_engine::storage::{is_plaintext_database, LocalStore, SqliteKey, SqliteLocalStore};
use gosub_engine::zone::ZoneId;
use std::path::Path;

fn key(n: u8) -> SqliteKey {
    SqliteKey::from_bytes([n; 32])
}

/// Encrypted on disk, and refused without the key or with another one.
fn assert_sealed(path: &Path, opens_with: impl Fn(Option<SqliteKey>) -> bool) {
    assert!(path.exists(), "{} not written", path.display());
    assert!(!is_plaintext_database(path).unwrap(), "{} is plaintext", path.display());
    assert!(!opens_with(None), "{} opened without a key", path.display());
    assert!(
        !opens_with(Some(key(9))),
        "{} opened with the wrong key",
        path.display()
    );
    assert!(
        opens_with(Some(key(1))),
        "{} did not reopen with its key",
        path.display()
    );
}

#[test]
fn settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.db");
    drop(SqliteStorageAdapter::open(&path, Some(&key(1))).unwrap());
    assert_sealed(&path, |k| SqliteStorageAdapter::open(&path, k.as_ref()).is_ok());
}

#[test]
fn cookies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cookies.db");
    drop(SqliteCookieStore::open(path.clone(), Some(key(1))).unwrap());
    assert_sealed(&path, |k| SqliteCookieStore::open(path.clone(), k).is_ok());
}

#[test]
fn places() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("places.db");
    let places = SqlitePlaces::open(path.clone(), Some(key(1))).unwrap();
    places.add_bookmark("https://example.org/", "Example");
    drop(places);
    assert_sealed(&path, |k| SqlitePlaces::open(path.clone(), k).is_ok());
    let places = SqlitePlaces::open(path.clone(), Some(key(1))).unwrap();
    assert_eq!(places.bookmarks().len(), 1);
}

#[test]
fn local_storage() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("local-storage.db");
    let path_str = path.to_str().unwrap();
    let store = SqliteLocalStore::open(path_str, Some(key(1))).unwrap();
    let origin = url::Url::parse("https://example.org/").unwrap().origin();
    let area = store.area(ZoneId::new(), &Default::default(), &origin).unwrap();
    area.set_item("k", "v").unwrap();
    drop((area, store));
    assert_sealed(&path, |k| SqliteLocalStore::open(path_str, k).is_ok());
}
