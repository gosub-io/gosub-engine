//! Cookie jar abstraction and a simple in-memory implementation.
//!
//! A **cookie jar** represents all cookies belonging to a single zone. The engine
//! passes request/response metadata to the jar so it can update and query cookies
//! appropriately.
//!
//! This module defines the [`CookieJar`] trait and a reference implementation,
//! [`DefaultCookieJar`], which stores cookies **in memory only** (no persistence)
//! and parses a subset of RFC 6265 `Set-Cookie` semantics.
//!
//! ## Notes & limitations
//! - Parsing is intentionally **minimal**: attributes like `Expires`, `Path`,
//!   `Domain`, `Secure`, `HttpOnly`, and `SameSite` are handled; `Max-Age`,
//!   priorities, size limits, eviction policies, and expiration enforcement are
//!   not (yet) implemented.
//! - Cookies are bucketed by **origin** (`url.origin().ascii_serialization()`).
//!   Within a bucket, simple host/subdomain and path prefix checks are applied.
//! - This module is **not** internally synchronized. Use it via a
//!   `CookieJarHandle = Arc<RwLock<dyn CookieJar + Send + Sync>>`.
//!
//! See also: RFC 6265bis (HTTP State Management Mechanism).
//!
use crate::engine::cookies::Cookie;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use url::Url;

/// A cookie jar keeps the cookies for one single zone.
///
/// Types implementing this trait should encapsulate storage, retrieval, and
/// mutation of cookies according to the URL/headers they receive.
///
/// ### Type erasure
/// `as_any` / `as_any_mut` enable downcasting when callers need access to
/// concrete implementations (e.g., for snapshotting/persistence).
pub trait CookieJar: Send + Sync {
    /// Returns a type-erased reference to the jar.
    fn as_any(&self) -> &dyn Any;

    /// Returns a mutable type-erased reference to the jar.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Stores cookies found in response `headers` for the given `url`.
    ///
    /// Implementations typically parse all `Set-Cookie` headers and update
    /// existing entries using "last write wins" semantics when names collide.
    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap);

    /// Returns the `Cookie` request header value to send for `url`, if any.
    ///
    /// Implementations should filter by domain, path, and the `Secure` flag.
    /// `None` means no cookies match the request.
    fn get_request_cookies(&self, url: &Url) -> Option<String>;

    /// Removes all cookies from the jar.
    fn clear(&mut self);

    /// Retrieves all cookies grouped by origin, formatted as `"name=value"` pairs.
    ///
    /// This is primarily intended for diagnostics/inspection.
    fn get_all_cookies(&self) -> Vec<(Url, String)>;

    /// Removes a single cookie with `cookie_name` associated with `url`.
    fn remove_cookie(&mut self, url: &Url, cookie_name: &str);

    /// Removes all cookies associated with `url` (bucketed by its origin).
    fn remove_cookies_for_url(&mut self, url: &Url);
}

/// Default cookie jar which holds cookies for a single zone.
///
/// This implementation is **in-memory only** and performs **no persistence**.
/// Cookies are stored per **origin** (`scheme://host:port`) and matched to
/// requests via basic domain/path rules.
///
/// ### Parsing behavior
/// - Accepts multiple `Set-Cookie` headers.
/// - Attributes handled: `Path`, `Domain` (leading dot stripped), `Expires`
///   (stored as raw string), `SameSite` (`Strict`/`Lax`/`None`, case-insensitive),
///   `Secure`, `HttpOnly`.
/// - If `Path` is absent, a default path is derived from the request URL.
/// - No expiration or eviction is enforced; `expires` is stored but not acted upon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultCookieJar {
    /// Simple hashmap of cookies, bucketed by **origin**.
    ///
    /// Key: origin string from `Url::origin().ascii_serialization()`.
    /// Value: vector of cookie records for that origin.
    pub entries: HashMap<String, Vec<Cookie>>,
}

impl DefaultCookieJar {
    /// Creates an empty in-memory cookie jar.
    pub fn new() -> Self {
        DefaultCookieJar {
            entries: HashMap::new(),
        }
    }
}

impl CookieJar for DefaultCookieJar {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap) {
        let origin = url.origin().ascii_serialization();
        let _host = url.host_str().unwrap_or_default();
        let default_path = url
            .path()
            .rsplit_once('/')
            .map_or("/", |(a, _)| if a.is_empty() { "/" } else { a });

        let bucket = self.entries.entry(origin).or_default();

        for header in headers.get_all("set-cookie") {
            if let Ok(header_str) = header.to_str() {
                if let Some((name, rest)) = header_str.split_once('=') {
                    let mut cookie = Cookie {
                        name: name.trim().to_string(),
                        value: String::new(),
                        path: None,
                        domain: None,
                        secure: false,
                        expires: None,
                        same_site: None,
                        http_only: false,
                    };

                    for part in rest.split(';') {
                        let part = part.trim();
                        if cookie.value.is_empty() {
                            cookie.value = part.to_string();
                            continue;
                        }

                        if let Some((k, v)) = part.split_once('=') {
                            match k.to_ascii_lowercase().as_str() {
                                "path" => cookie.path = Some(v.to_string()),
                                "domain" => cookie.domain = Some(v.trim_start_matches('.').to_string()),
                                "expires" => cookie.expires = Some(v.to_string()),
                                "samesite" => {
                                    // normalize to "Lax" | "Strict" | "None"
                                    let val = v.trim();
                                    if val.eq_ignore_ascii_case("lax") {
                                        cookie.same_site = Some("Lax".to_string());
                                    } else if val.eq_ignore_ascii_case("strict") {
                                        cookie.same_site = Some("Strict".to_string());
                                    } else if val.eq_ignore_ascii_case("none") {
                                        cookie.same_site = Some("None".to_string());
                                        // Optional hardening: SameSite=None SHOULD be Secure.
                                        // If you want to enforce it, uncomment the next line.
                                        // if !cookie.secure { cookie.secure = true; }
                                    } else {
                                        // leave as-is if unknown, or set Some(val.to_string())
                                        cookie.same_site = Some(val.to_string());
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            if part.eq_ignore_ascii_case("secure") {
                                cookie.secure = true;
                            } else if part.eq_ignore_ascii_case("httponly") {
                                cookie.http_only = true;
                            }
                        }
                    }

                    if cookie.path.is_none() {
                        cookie.path = Some(default_path.to_string());
                    }

                    // Replace existing cookie with same name
                    if let Some(existing) = bucket.iter_mut().find(|c| c.name == cookie.name) {
                        *existing = cookie;
                    } else {
                        bucket.push(cookie);
                    }
                }
            }
        }
    }

    fn get_request_cookies(&self, url: &Url) -> Option<String> {
        let origin = url.origin().ascii_serialization();
        let host = url.host_str().unwrap_or_default();
        let path = url.path();
        let is_https = url.scheme() == "https";

        let cookies = self.entries.get(&origin)?;

        let header = cookies
            .iter()
            .filter(|cookie| {
                // Check domain match
                match &cookie.domain {
                    Some(domain) => host == domain || host.ends_with(&format!(".{}", domain)),
                    None => true,
                }
            })
            .filter(|cookie| {
                // Check path match
                match &cookie.path {
                    Some(cookie_path) => path.starts_with(cookie_path),
                    None => true,
                }
            })
            .filter(|cookie| {
                // Check secure
                !cookie.secure || is_https
            })
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");

        if header.is_empty() {
            None
        } else {
            Some(header)
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn get_all_cookies(&self) -> Vec<(Url, String)> {
        self.entries
            .iter()
            .filter_map(|(origin, cookies)| {
                Url::parse(origin).ok().map(|url| {
                    let str_ = cookies
                        .iter()
                        .map(|c| format!("{}={}", c.name, c.value))
                        .collect::<Vec<_>>()
                        .join("; ");
                    (url, str_)
                })
            })
            .collect()
    }

    fn remove_cookie(&mut self, url: &Url, cookie_name: &str) {
        let origin = url.origin().ascii_serialization();
        if let Some(cookies) = self.entries.get_mut(&origin) {
            cookies.retain(|c| c.name != cookie_name);
        }
    }

    fn remove_cookies_for_url(&mut self, url: &Url) {
        let origin = url.origin().ascii_serialization();
        self.entries.remove(&origin);
    }
}
