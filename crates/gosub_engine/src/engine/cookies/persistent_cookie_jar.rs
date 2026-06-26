use crate::engine::cookies::cookie_jar::{DefaultCookieJar, SameSiteContext};
use crate::engine::cookies::{CookieJar, CookieJarHandle, CookieStoreHandle};
use crate::engine::zone::ZoneId;
use http::HeaderMap;
use url::Url;

/// A `CookieJar` decorator that persists changes after each mutation.
///
/// This type is *transparent* for reads but *eagerly* persists after writes.
pub struct PersistentCookieJar {
    /// Zone ID associated with this jar (used to address the store).
    zone_id: ZoneId,
    /// Inner cookie jar that holds the actual cookie state.
    pub inner: CookieJarHandle,
    /// Handle to the cookie store responsible for persistence.
    store_handle: CookieStoreHandle,
}

impl PersistentCookieJar {
    /// Creates a new persistence-enabled wrapper around an existing jar.
    ///
    /// The `store` will be used to persist snapshots after each mutation.
    pub fn new(zone_id: ZoneId, jar: CookieJarHandle, store_handle: CookieStoreHandle) -> Self {
        Self {
            zone_id,
            inner: jar,
            store_handle,
        }
    }

    /// Snapshots the inner jar and persists it to the backing store.
    ///
    /// Persistence is best-effort: if the inner jar is not a [`DefaultCookieJar`]
    /// (the downcast is required to obtain a cloneable snapshot), the snapshot is
    /// skipped and an error is logged.
    fn persist(&self) {
        // Create a snapshot of the current state of the cookie jar. This is what we will store with "persist()"
        let snapshot = {
            let inner = self.inner.read();
            let Some(jar) = inner.as_any().downcast_ref::<DefaultCookieJar>() else {
                log::error!("Inner jar is not a DefaultCookieJar; skipping cookie persistence");
                return;
            };
            jar.clone()
        };

        self.store_handle.persist_zone_from_snapshot(self.zone_id, &snapshot);
    }
}

impl CookieJar for PersistentCookieJar {
    /// Returns a type-erased reference to this jar (the wrapper itself).
    /// @TODO: check if we still need these.
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    /// Stores cookies from a response, then persists the updated state.
    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap, top_level: Option<&Url>) {
        self.inner.write().store_response_cookies(url, headers, top_level);
        self.persist();
    }

    /// Returns the `Cookie` request header value for `url` without persisting.
    fn get_request_cookies(&self, url: &Url, top_level: Option<&Url>, samesite: SameSiteContext) -> Option<String> {
        self.inner.read().get_request_cookies(url, top_level, samesite)
    }

    /// Clears all cookies in the jar, then persists the updated state.
    fn clear(&mut self) {
        self.inner.write().clear();
        self.persist();
    }

    /// Returns all cookies (for debugging/inspection) without persisting.
    fn get_all_cookies(&self) -> Vec<(Url, String)> {
        self.inner.read().get_all_cookies()
    }

    /// Removes a single cookie by name for `url`, then persists the updated state.
    fn remove_cookie(&mut self, url: &Url, cookie_name: &str) {
        self.inner.write().remove_cookie(url, cookie_name);
        self.persist();
    }

    /// Removes all cookies for `url`, then persists the updated state.
    fn remove_cookies_for_url(&mut self, url: &Url) {
        self.inner.write().remove_cookies_for_url(url);
        self.persist();
    }

    /// Removes expired cookies from the inner jar, then persists the result.
    fn purge_expired(&mut self) {
        self.inner.write().purge_expired();
        self.persist();
    }
}
