use crate::cookies::cookie_jar::DefaultCookieJar;
use crate::cookies::{CookieJar, CookieJarHandle, CookieStoreHandle};
use gosub_net::types::ZoneId;
use http::HeaderMap;
use url::Url;

pub struct PersistentCookieJar {
    zone_id: ZoneId,
    pub inner: CookieJarHandle,
    store_handle: CookieStoreHandle,
}

impl PersistentCookieJar {
    pub fn new(zone_id: ZoneId, jar: CookieJarHandle, store_handle: CookieStoreHandle) -> Self {
        Self { zone_id, inner: jar, store_handle }
    }

    fn persist(&self) {
        let snapshot = {
            let inner = self.inner.read();
            let jar = inner
                .as_any()
                .downcast_ref::<DefaultCookieJar>()
                .expect("inner must be DefaultCookieJar");
            jar.clone()
        };
        self.store_handle.persist_zone_from_snapshot(self.zone_id, &snapshot);
    }
}

impl CookieJar for PersistentCookieJar {
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap) {
        self.inner.write().store_response_cookies(url, headers);
        self.persist();
    }

    fn get_request_cookies(&self, url: &Url) -> Option<String> {
        self.inner.read().get_request_cookies(url)
    }

    fn clear(&mut self) {
        self.inner.write().clear();
        self.persist();
    }

    fn get_all_cookies(&self) -> Vec<(Url, String)> {
        self.inner.read().get_all_cookies()
    }

    fn remove_cookie(&mut self, url: &Url, cookie_name: &str) {
        self.inner.write().remove_cookie(url, cookie_name);
        self.persist();
    }

    fn remove_cookies_for_url(&mut self, url: &Url) {
        self.inner.write().remove_cookies_for_url(url);
        self.persist();
    }
}
