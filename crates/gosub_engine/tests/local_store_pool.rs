//! Opening a `SqliteLocalStore` must not make its connection pool log errors.
//!
//! The pool opens all its connections up front, and r2d2 logs every failed attempt as a
//! bare `error!("{}", e)` before it retries, which a shell prints as `[E] database is
//! locked` with nothing to say where it came from. This file installs its own logger, so it
//! is a test binary of its own.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use gosub_engine::storage::SqliteLocalStore;
use parking_lot::Mutex;
use r2d2_sqlite::rusqlite::Connection;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;
use std::time::Duration;

static ERRORS: AtomicUsize = AtomicUsize::new(0);
static FIRST: Mutex<Option<String>> = Mutex::new(None);

struct CountErrors;

impl log::Log for CountErrors {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() == log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if record.level() == log::Level::Error {
            ERRORS.fetch_add(1, Ordering::SeqCst);
            FIRST
                .lock()
                .get_or_insert_with(|| format!("{}: {}", record.target(), record.args()));
        }
    }

    fn flush(&self) {}
}

fn install_logger() {
    static LOGGER: CountErrors = CountErrors;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Error);
}

fn logged_errors() -> (usize, Option<String>) {
    (ERRORS.load(Ordering::SeqCst), FIRST.lock().clone())
}

/// One test, so the global error count belongs to it alone.
#[test]
fn opening_the_store_logs_no_pool_errors() {
    install_logger();
    let dir = tempfile::tempdir().unwrap();

    // Fresh files: every connection of a new pool sets the database up at once.
    for i in 0..20 {
        let path = dir.path().join(format!("fresh-{i}.db"));
        SqliteLocalStore::new(path.to_str().unwrap()).unwrap();
    }

    // Another process holding the write lock for longer than a single busy wait.
    let path = dir.path().join("held.db");
    SqliteLocalStore::new(path.to_str().unwrap()).unwrap();
    let held = Barrier::new(2);
    std::thread::scope(|s| {
        s.spawn(|| {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
            held.wait();
            std::thread::sleep(Duration::from_millis(800));
            conn.execute_batch("COMMIT").unwrap();
        });
        held.wait();
        SqliteLocalStore::new(path.to_str().unwrap()).unwrap();
    });

    assert_eq!(logged_errors(), (0, None));
}
