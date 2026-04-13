mod in_memory;
mod json;
#[cfg(feature = "sqlite_cookie_store")]
mod sqlite;

use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::cookies::CookieJarHandle;
use gosub_net::types::ZoneId;

pub use in_memory::InMemoryCookieStore;
#[cfg(not(feature = "sqlite_cookie_store"))]
pub use in_memory::InMemoryCookieStore as SqliteCookieStore;
pub use json::JsonCookieStore;
#[cfg(feature = "sqlite_cookie_store")]
pub use sqlite::SqliteCookieStore;

pub trait CookieStore: Send + Sync {
    fn jar_for(&self, zone_id: ZoneId) -> Option<CookieJarHandle>;
    fn persist_zone_from_snapshot(&self, zone_id: ZoneId, snapshot: &DefaultCookieJar);
    fn remove_zone(&self, zone_id: ZoneId);
    fn persist_all(&self);
}
