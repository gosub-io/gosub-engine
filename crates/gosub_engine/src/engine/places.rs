//! "Places": per-zone bookmarks and global visited history.
//!
//! One store per zone (profile), handed in via [`ZoneServices`](crate::zone::ZoneServices)
//! like the cookie store, so every shell shares one format. The engine records a visit on
//! every committed http(s) navigation; everything else - bookmark management, history
//! queries for URL-bar autocompletion - is driven by the embedder through the same
//! [`PlacesHandle`] it constructed the store with. Queries are synchronous and cheap, so
//! shells can call them per keystroke.
//!
//! Two implementations: [`SqlitePlaces`] (persistent, behind the `sqlite_places` feature)
//! and [`MemoryPlaces`] (ephemeral zones and tests).

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A bookmarked page. One bookmark per URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    /// Unix seconds when the bookmark was created.
    pub created_at: u64,
}

/// A visited page, aggregated over all its visits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisitedPage {
    pub url: String,
    pub title: String,
    pub visit_count: u64,
    /// Unix seconds of the most recent visit.
    pub last_visit: u64,
}

/// Bookmark and history storage for one zone.
pub trait Places: Send + Sync + std::fmt::Debug {
    /// Add (or update the title of) a bookmark for `url`.
    fn add_bookmark(&self, url: &str, title: &str);
    fn remove_bookmark(&self, url: &str);
    fn is_bookmarked(&self, url: &str) -> bool;
    /// All bookmarks, oldest first.
    fn bookmarks(&self) -> Vec<Bookmark>;

    /// Record one visit of `url` (called by the engine on committed navigations).
    fn record_visit(&self, url: &str, title: &str);
    /// Visited pages whose URL or title contains `query` (case-insensitive), most
    /// visited first, then most recent. An empty query returns the most visited pages.
    fn query_visited(&self, query: &str, limit: usize) -> Vec<VisitedPage>;
    fn clear_history(&self);
}

pub type PlacesHandle = Arc<dyn Places>;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── In-memory implementation ────────────────────────────────────────────────

/// Volatile places store: ephemeral (private) zones and tests.
#[derive(Default, Debug)]
pub struct MemoryPlaces {
    inner: parking_lot::Mutex<MemoryInner>,
}

#[derive(Default, Debug)]
struct MemoryInner {
    bookmarks: Vec<Bookmark>,
    visits: Vec<VisitedPage>,
}

impl MemoryPlaces {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Places for MemoryPlaces {
    fn add_bookmark(&self, url: &str, title: &str) {
        let mut inner = self.inner.lock();
        if let Some(existing) = inner.bookmarks.iter_mut().find(|b| b.url == url) {
            existing.title = title.to_string();
            return;
        }
        inner.bookmarks.push(Bookmark {
            url: url.to_string(),
            title: title.to_string(),
            created_at: now_secs(),
        });
    }

    fn remove_bookmark(&self, url: &str) {
        self.inner.lock().bookmarks.retain(|b| b.url != url);
    }

    fn is_bookmarked(&self, url: &str) -> bool {
        self.inner.lock().bookmarks.iter().any(|b| b.url == url)
    }

    fn bookmarks(&self) -> Vec<Bookmark> {
        self.inner.lock().bookmarks.clone()
    }

    fn record_visit(&self, url: &str, title: &str) {
        let mut inner = self.inner.lock();
        if let Some(existing) = inner.visits.iter_mut().find(|v| v.url == url) {
            existing.visit_count += 1;
            existing.last_visit = now_secs();
            if !title.is_empty() {
                existing.title = title.to_string();
            }
            return;
        }
        inner.visits.push(VisitedPage {
            url: url.to_string(),
            title: title.to_string(),
            visit_count: 1,
            last_visit: now_secs(),
        });
    }

    fn query_visited(&self, query: &str, limit: usize) -> Vec<VisitedPage> {
        use cow_utils::CowUtils;
        let query = query.cow_to_lowercase();
        let query = query.as_ref();
        let mut hits: Vec<VisitedPage> = self
            .inner
            .lock()
            .visits
            .iter()
            .filter(|v| {
                query.is_empty()
                    || v.url.cow_to_lowercase().contains(query)
                    || v.title.cow_to_lowercase().contains(query)
            })
            .cloned()
            .collect();
        hits.sort_by_key(|v| std::cmp::Reverse((v.visit_count, v.last_visit)));
        hits.truncate(limit);
        hits
    }

    fn clear_history(&self) {
        self.inner.lock().visits.clear();
    }
}

// ── SQLite implementation ───────────────────────────────────────────────────

#[cfg(feature = "sqlite_places")]
pub use sqlite::SqlitePlaces;

#[cfg(feature = "sqlite_places")]
mod sqlite {
    use super::{now_secs, Bookmark, Places, VisitedPage};
    use crate::EngineError;
    use r2d2::Pool;
    use r2d2_sqlite::rusqlite::params;
    use r2d2_sqlite::SqliteConnectionManager;
    use std::path::PathBuf;

    /// Persistent places store, one SQLite database per zone. Access goes through an
    /// `r2d2` pool, like the cookie store; operations log-and-degrade rather than
    /// propagate errors, so a broken history database never breaks browsing.
    pub struct SqlitePlaces {
        pool: Pool<SqliteConnectionManager>,
    }

    impl std::fmt::Debug for SqlitePlaces {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SqlitePlaces").finish_non_exhaustive()
        }
    }

    impl SqlitePlaces {
        pub fn new(path: PathBuf) -> Result<Self, EngineError> {
            let manager = SqliteConnectionManager::file(path);
            let pool = Pool::new(manager).map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
            {
                let conn = pool.get().map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS bookmarks (
                         url        TEXT PRIMARY KEY,
                         title      TEXT NOT NULL,
                         created_at INTEGER NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS visits (
                         url         TEXT PRIMARY KEY,
                         title       TEXT NOT NULL,
                         visit_count INTEGER NOT NULL,
                         last_visit  INTEGER NOT NULL
                     );",
                )
                .map_err(|e| EngineError::Internal(anyhow::anyhow!(e)))?;
            }
            Ok(Self { pool })
        }

        /// Escape SQL LIKE wildcards in user input (`ESCAPE '\'` in the queries).
        fn like_pattern(query: &str) -> String {
            let mut escaped = String::with_capacity(query.len() + 2);
            escaped.push('%');
            for c in query.chars() {
                if matches!(c, '\\' | '%' | '_') {
                    escaped.push('\\');
                }
                escaped.push(c);
            }
            escaped.push('%');
            escaped
        }
    }

    impl Places for SqlitePlaces {
        fn add_bookmark(&self, url: &str, title: &str) {
            let Ok(conn) = self.pool.get() else { return };
            let _ = conn
                .execute(
                    "INSERT INTO bookmarks (url, title, created_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(url) DO UPDATE SET title = ?2",
                    params![url, title, now_secs() as i64],
                )
                .map_err(|e| log::warn!("places: add_bookmark failed: {e}"));
        }

        fn remove_bookmark(&self, url: &str) {
            let Ok(conn) = self.pool.get() else { return };
            let _ = conn
                .execute("DELETE FROM bookmarks WHERE url = ?1", params![url])
                .map_err(|e| log::warn!("places: remove_bookmark failed: {e}"));
        }

        fn is_bookmarked(&self, url: &str) -> bool {
            let Ok(conn) = self.pool.get() else { return false };
            conn.query_row("SELECT 1 FROM bookmarks WHERE url = ?1", params![url], |_| Ok(()))
                .is_ok()
        }

        fn bookmarks(&self) -> Vec<Bookmark> {
            let Ok(conn) = self.pool.get() else { return Vec::new() };
            let Ok(mut stmt) = conn.prepare("SELECT url, title, created_at FROM bookmarks ORDER BY created_at, url")
            else {
                return Vec::new();
            };
            stmt.query_map([], |row| {
                Ok(Bookmark {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get::<_, i64>(2)? as u64,
                })
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
        }

        fn record_visit(&self, url: &str, title: &str) {
            let Ok(conn) = self.pool.get() else { return };
            let _ = conn
                .execute(
                    "INSERT INTO visits (url, title, visit_count, last_visit) VALUES (?1, ?2, 1, ?3)
                     ON CONFLICT(url) DO UPDATE SET
                         visit_count = visit_count + 1,
                         last_visit = ?3,
                         title = CASE WHEN length(?2) > 0 THEN ?2 ELSE title END",
                    params![url, title, now_secs() as i64],
                )
                .map_err(|e| log::warn!("places: record_visit failed: {e}"));
        }

        fn query_visited(&self, query: &str, limit: usize) -> Vec<VisitedPage> {
            let Ok(conn) = self.pool.get() else { return Vec::new() };
            let pattern = Self::like_pattern(query);
            let Ok(mut stmt) = conn.prepare(
                "SELECT url, title, visit_count, last_visit FROM visits
                 WHERE ?1 = '%%' OR url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY visit_count DESC, last_visit DESC
                 LIMIT ?2",
            ) else {
                return Vec::new();
            };
            stmt.query_map(params![pattern, limit as i64], |row| {
                Ok(VisitedPage {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    visit_count: row.get::<_, i64>(2)? as u64,
                    last_visit: row.get::<_, i64>(3)? as u64,
                })
            })
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
        }

        fn clear_history(&self) {
            let Ok(conn) = self.pool.get() else { return };
            let _ = conn
                .execute("DELETE FROM visits", [])
                .map_err(|e| log::warn!("places: clear_history failed: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stores() -> Vec<(&'static str, PlacesHandle)> {
        #[allow(unused_mut)] // mut is used only when the sqlite_places feature is on
        let mut out: Vec<(&'static str, PlacesHandle)> = vec![("memory", Arc::new(MemoryPlaces::new()))];
        #[cfg(feature = "sqlite_places")]
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("places.db");
            // Leak the tempdir so the database outlives this helper for the test's duration.
            std::mem::forget(dir);
            out.push(("sqlite", Arc::new(SqlitePlaces::new(path).unwrap())));
        }
        out
    }

    #[test]
    fn bookmarks_roundtrip() {
        for (name, store) in stores() {
            assert!(!store.is_bookmarked("https://a.example/"), "{name}");
            store.add_bookmark("https://a.example/", "A");
            store.add_bookmark("https://b.example/", "B");
            assert!(store.is_bookmarked("https://a.example/"), "{name}");
            // Re-adding updates the title, no duplicate.
            store.add_bookmark("https://a.example/", "A2");
            let list = store.bookmarks();
            assert_eq!(list.len(), 2, "{name}");
            assert_eq!(list.iter().find(|b| b.url == "https://a.example/").unwrap().title, "A2");
            store.remove_bookmark("https://a.example/");
            assert!(!store.is_bookmarked("https://a.example/"), "{name}");
            assert_eq!(store.bookmarks().len(), 1, "{name}");
        }
    }

    #[test]
    fn visits_aggregate_and_query() {
        for (name, store) in stores() {
            store.record_visit("https://news.example/", "News site");
            store.record_visit("https://news.example/", "News site");
            store.record_visit("https://docs.example/page", "Documentation");

            let all = store.query_visited("", 10);
            assert_eq!(all.len(), 2, "{name}");
            assert_eq!(all[0].url, "https://news.example/", "{name}: most visited first");
            assert_eq!(all[0].visit_count, 2, "{name}");

            // Substring match on URL and on title, case-insensitive.
            assert_eq!(store.query_visited("DOCS", 10).len(), 1, "{name}");
            assert_eq!(store.query_visited("news site", 10).len(), 1, "{name}");
            assert_eq!(store.query_visited("nothing", 10).len(), 0, "{name}");
            // LIKE wildcards in user input match literally, not as wildcards.
            assert_eq!(store.query_visited("%", 10).len(), 0, "{name}");

            store.clear_history();
            assert_eq!(store.query_visited("", 10).len(), 0, "{name}");
        }
    }
}
