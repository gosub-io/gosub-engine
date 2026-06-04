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
use chrono::Utc;
use cow_utils::CowUtils;
use http::HeaderMap;
use once_cell::sync::Lazy;
use publicsuffix::{List, Psl as _};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use url::Url;

static PSL: Lazy<List> = Lazy::new(List::new);

/// Parse an HTTP date string (RFC 1123 / RFC 2822) into a Unix timestamp.
///
/// Returns `None` if the string cannot be parsed, which causes the cookie to be
/// treated as a session cookie rather than silently accepting a bad expiry.
fn parse_http_date(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(s)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Returns `true` when two hostnames share the same registrable domain (eTLD+1).
///
/// Uses the Mozilla Public Suffix List for accurate comparison. Falls back to
/// exact hostname equality for IP addresses, `localhost`, and other hosts that
/// the PSL cannot parse.
fn same_site(host_a: &str, host_b: &str) -> bool {
    let registrable = |host: &str| -> Option<String> {
        let d = PSL.domain(host.as_bytes())?;
        std::str::from_utf8(d.as_bytes()).ok().map(str::to_owned)
    };
    match (registrable(host_a), registrable(host_b)) {
        (Some(a), Some(b)) => a == b,
        _ => host_a == host_b,
    }
}

/// Controls how the jar handles cookies in cross-site (third-party) request contexts.
///
/// Applied by [`DefaultCookieJar::get_request_cookies`] and
/// [`DefaultCookieJar::store_response_cookies`] when a `top_level` URL is supplied
/// and its registrable domain differs from the request URL's registrable domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThirdPartyCookiePolicy {
    /// All cookies are sent/stored regardless of cross-site context. (Default; matches
    /// legacy browser behavior.)
    #[default]
    Allow,
    /// No cookies are sent or stored in third-party context.
    Block,
    /// In third-party context only cookies with `SameSite=None; Secure` are sent or
    /// stored. All others are blocked.
    SameSiteNoneOnly,
}

/// Per-request context for `SameSite` cookie attribute enforcement.
///
/// The HTTP layer computes the correct variant from the navigation type and HTTP
/// method, then passes it to [`CookieJar::get_request_cookies`].
///
/// RFC 6265bis rules applied by [`DefaultCookieJar`]:
///
/// | Cookie attribute | `SameSite` | `CrossSiteNavigation` | `CrossSite` |
/// |---|:---:|:---:|:---:|
/// | `SameSite=Strict`   | ✓ | ✗ | ✗ |
/// | `SameSite=Lax`      | ✓ | ✓ | ✗ |
/// | *(no attribute)*    | ✓ | ✓ | ✗ |
/// | `SameSite=None; Secure` | ✓ | ✓ | ✓ |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SameSiteContext {
    /// Same-site request. All `SameSite` attribute values are eligible.
    #[default]
    SameSite,
    /// Cross-site top-level navigation initiated by a safe HTTP method (GET, HEAD).
    /// `Lax` and `None` cookies are included; `Strict` cookies are blocked.
    CrossSiteNavigation,
    /// Cross-site subrequest (image, fetch, iframe, etc.) or a cross-site navigation
    /// with an unsafe method (POST, PUT, …).
    /// Only `SameSite=None; Secure` cookies are eligible.
    CrossSite,
}

/// A cookie jar keeps the cookies for one single zone.
///
/// Types implementing this trait should encapsulate storage, retrieval, and
/// mutation of cookies according to the URL/headers they receive.
///
/// ### Third-party context
/// Both `store_response_cookies` and `get_request_cookies` accept an optional
/// `top_level` URL representing the page that initiated the request. When present,
/// implementations can use it to distinguish first-party from third-party requests
/// and apply the appropriate cookie policy.
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
    /// `top_level` is the URL of the page that triggered the request (tab's current
    /// URL). When `Some`, implementations may enforce third-party cookie policy
    /// (e.g., block storage when the request is cross-site).
    ///
    /// Implementations typically parse all `Set-Cookie` headers and update
    /// existing entries using "last write wins" semantics when names collide.
    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap, top_level: Option<&Url>);

    /// Returns the `Cookie` request header value to send for `url`, if any.
    ///
    /// `top_level` is the URL of the page that triggered the request (tab's current
    /// URL). When `Some`, implementations should enforce third-party cookie policy.
    ///
    /// `samesite` encodes the request's cross-site context and drives enforcement of
    /// the `SameSite` cookie attribute per RFC 6265bis.
    ///
    /// Implementations should also filter by domain, path, and the `Secure` flag.
    /// Returns `None` when no cookies match the request.
    fn get_request_cookies(&self, url: &Url, top_level: Option<&Url>, samesite: SameSiteContext) -> Option<String>;

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

    /// Removes all cookies whose expiry timestamp is in the past.
    ///
    /// Session cookies (`expires == None`) are never removed by this call.
    /// Useful on jar load and for periodic cleanup to bound memory growth.
    fn purge_expired(&mut self);
}

/// Default cookie jar which holds cookies for a single zone.
///
/// This implementation is **in-memory only** and performs **no persistence**.
/// Cookies are stored per **origin** (`scheme://host:port`) and matched to
/// requests via basic domain/path rules.
///
/// ### Third-party policy
/// When `top_level` is provided to `get_request_cookies` or `store_response_cookies`,
/// the `third_party_policy` field controls cross-site behavior:
/// - `Allow` — legacy behavior, all cookies pass through.
/// - `Block` — no cookies are sent or stored for third-party requests.
/// - `SameSiteNoneOnly` — only `SameSite=None; Secure` cookies are allowed in
///   third-party context.
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

    /// Policy applied when a cross-site `top_level` URL is detected.
    pub third_party_policy: ThirdPartyCookiePolicy,
}

impl Default for DefaultCookieJar {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultCookieJar {
    /// Creates an empty in-memory cookie jar with `ThirdPartyCookiePolicy::Allow`.
    pub fn new() -> Self {
        DefaultCookieJar {
            entries: HashMap::new(),
            third_party_policy: ThirdPartyCookiePolicy::default(),
        }
    }

    /// Returns a new jar with the given third-party cookie policy.
    pub fn with_policy(mut self, policy: ThirdPartyCookiePolicy) -> Self {
        self.third_party_policy = policy;
        self
    }
}

impl CookieJar for DefaultCookieJar {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn store_response_cookies(&mut self, url: &Url, headers: &HeaderMap, top_level: Option<&Url>) {
        // Determine cross-site context before touching storage.
        if let Some(tl) = top_level {
            let req_host = url.host_str().unwrap_or_default();
            let tl_host = tl.host_str().unwrap_or_default();
            if !same_site(req_host, tl_host) {
                match self.third_party_policy {
                    ThirdPartyCookiePolicy::Allow => {}
                    ThirdPartyCookiePolicy::Block => return,
                    // SameSiteNoneOnly is handled per-cookie below.
                    ThirdPartyCookiePolicy::SameSiteNoneOnly => {}
                }
            }
        }

        let is_third_party = top_level.is_some_and(|tl| {
            let req_host = url.host_str().unwrap_or_default();
            let tl_host = tl.host_str().unwrap_or_default();
            !same_site(req_host, tl_host)
        });

        let origin = url.origin().ascii_serialization();
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

                    // Collected during attribute parsing; resolved after the loop.
                    let mut max_age: Option<i64> = None;
                    let mut expires_str: Option<String> = None;

                    for part in rest.split(';') {
                        let part = part.trim();
                        if cookie.value.is_empty() {
                            cookie.value = part.to_string();
                            continue;
                        }

                        if let Some((k, v)) = part.split_once('=') {
                            match k.cow_to_ascii_lowercase().as_ref() {
                                "path" => cookie.path = Some(v.to_string()),
                                "domain" => cookie.domain = Some(v.trim_start_matches('.').to_string()),
                                "expires" => expires_str = Some(v.to_string()),
                                "max-age" => max_age = v.trim().parse::<i64>().ok(),
                                "samesite" => {
                                    let val = v.trim();
                                    if val.eq_ignore_ascii_case("lax") {
                                        cookie.same_site = Some("Lax".to_string());
                                    } else if val.eq_ignore_ascii_case("strict") {
                                        cookie.same_site = Some("Strict".to_string());
                                    } else if val.eq_ignore_ascii_case("none") {
                                        cookie.same_site = Some("None".to_string());
                                    } else {
                                        cookie.same_site = Some(val.to_string());
                                    }
                                }
                                _ => {}
                            }
                        } else if part.eq_ignore_ascii_case("secure") {
                            cookie.secure = true;
                        } else if part.eq_ignore_ascii_case("httponly") {
                            cookie.http_only = true;
                        }
                    }

                    if cookie.path.is_none() {
                        cookie.path = Some(default_path.to_string());
                    }

                    // Resolve expiry: Max-Age takes precedence over Expires (RFC 6265 §5.2).
                    let now = Utc::now().timestamp();
                    cookie.expires = if let Some(ma) = max_age {
                        if ma <= 0 {
                            // Max-Age=0 (or negative) means delete the cookie immediately.
                            bucket.retain(|c| c.name != cookie.name);
                            continue;
                        }
                        Some(now + ma)
                    } else {
                        expires_str.as_deref().and_then(parse_http_date)
                    };

                    // In SameSiteNoneOnly mode, drop third-party cookies that don't
                    // carry SameSite=None; Secure.
                    if is_third_party
                        && self.third_party_policy == ThirdPartyCookiePolicy::SameSiteNoneOnly
                        && !(matches!(cookie.same_site.as_deref(), Some("None")) && cookie.secure)
                    {
                        continue;
                    }

                    if let Some(existing) = bucket.iter_mut().find(|c| c.name == cookie.name) {
                        *existing = cookie;
                    } else {
                        bucket.push(cookie);
                    }
                }
            }
        }
    }

    fn get_request_cookies(&self, url: &Url, top_level: Option<&Url>, samesite: SameSiteContext) -> Option<String> {
        // Apply third-party policy when a top-level URL is provided.
        let is_third_party = top_level.is_some_and(|tl| {
            let req_host = url.host_str().unwrap_or_default();
            let tl_host = tl.host_str().unwrap_or_default();
            !same_site(req_host, tl_host)
        });

        if is_third_party {
            match self.third_party_policy {
                ThirdPartyCookiePolicy::Allow => {}
                ThirdPartyCookiePolicy::Block => return None,
                ThirdPartyCookiePolicy::SameSiteNoneOnly => {} // filtered per-cookie below
            }
        }

        let origin = url.origin().ascii_serialization();
        let host = url.host_str().unwrap_or_default();
        let path = url.path();
        let is_https = url.scheme() == "https";

        let cookies = self.entries.get(&origin)?;

        let now = Utc::now().timestamp();

        let header = cookies
            .iter()
            .filter(|cookie| {
                // Drop expired cookies (session cookies have expires == None).
                cookie.expires.map_or(true, |exp| exp > now)
            })
            .filter(|cookie| {
                // Third-party policy: SameSiteNoneOnly allows only None+Secure cross-site.
                if is_third_party && self.third_party_policy == ThirdPartyCookiePolicy::SameSiteNoneOnly {
                    return matches!(cookie.same_site.as_deref(), Some("None")) && cookie.secure;
                }
                true
            })
            .filter(|cookie| {
                // SameSite attribute enforcement (RFC 6265bis).
                // Cookies with no SameSite attribute default to Lax behavior.
                match cookie.same_site.as_deref() {
                    Some("Strict") => samesite == SameSiteContext::SameSite,
                    Some("None") => cookie.secure,
                    // Lax (explicit) or absent (implicit Lax default)
                    _ => matches!(samesite, SameSiteContext::SameSite | SameSiteContext::CrossSiteNavigation),
                }
            })
            .filter(|cookie| match &cookie.domain {
                Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
                None => true,
            })
            .filter(|cookie| match &cookie.path {
                Some(cookie_path) => path.starts_with(cookie_path),
                None => true,
            })
            .filter(|cookie| !cookie.secure || is_https)
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

    fn purge_expired(&mut self) {
        let now = Utc::now().timestamp();
        for cookies in self.entries.values_mut() {
            cookies.retain(|c| c.expires.map_or(true, |exp| exp > now));
        }
        self.entries.retain(|_, v| !v.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;

    fn headers(set_cookie: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for &v in set_cookie {
            map.append("set-cookie", v.parse().unwrap());
        }
        map
    }

    fn url(s: &str) -> Url {
        s.parse().unwrap()
    }

    // ── same_site helper ─────────────────────────────────────────────────────

    #[test]
    fn same_site_identical_hosts() {
        assert!(same_site("example.com", "example.com"));
    }

    #[test]
    fn same_site_subdomains_share_registrable() {
        assert!(same_site("foo.example.com", "bar.example.com"));
    }

    #[test]
    fn same_site_different_registrable_domains() {
        assert!(!same_site("example.com", "other.com"));
    }

    #[test]
    fn same_site_fallback_for_localhost() {
        assert!(same_site("localhost", "localhost"));
        assert!(!same_site("localhost", "127.0.0.1"));
    }

    // ── ThirdPartyCookiePolicy::Allow (default) ───────────────────────────────

    #[test]
    fn allow_policy_first_party_returns_cookie() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/page");
        let h = headers(&["id=1; Path=/"]);
        jar.store_response_cookies(&req, &h, None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("id=1")
        );
    }

    #[test]
    fn allow_policy_third_party_sends_cookie() {
        let mut jar = DefaultCookieJar::new();
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        // Plain cookie with no SameSite attribute → defaults to Lax.
        // ThirdPartyCookiePolicy::Allow does not add an extra restriction,
        // so the cookie passes on a cross-site navigation (Lax allows that).
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert_eq!(
            jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSiteNavigation).as_deref(),
            Some("uid=x")
        );
        // The same cookie is still blocked on a cross-site subrequest (Lax default).
        assert!(jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite).is_none());
    }

    // ── ThirdPartyCookiePolicy::Block ─────────────────────────────────────────

    #[test]
    fn block_policy_prevents_third_party_storage() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert!(jar.get_request_cookies(&resource, None, SameSiteContext::SameSite).is_none());
    }

    #[test]
    fn block_policy_prevents_third_party_sending() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        // Store as first-party (no top_level), then try to send as third-party.
        let resource = url("https://tracker.com/pixel");
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, None);

        let top = url("https://example.com/");
        assert!(jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite).is_none());
    }

    #[test]
    fn block_policy_allows_first_party() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        let req = url("https://example.com/page");
        let top = url("https://example.com/");
        let h = headers(&["id=1; Path=/"]);
        jar.store_response_cookies(&req, &h, Some(&top));

        assert_eq!(
            jar.get_request_cookies(&req, Some(&top), SameSiteContext::SameSite).as_deref(),
            Some("id=1")
        );
    }

    // ── ThirdPartyCookiePolicy::SameSiteNoneOnly ──────────────────────────────

    #[test]
    fn samesite_none_only_blocks_plain_third_party_cookie() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::SameSiteNoneOnly);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        // No SameSite=None; Secure — should be blocked.
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert!(jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite).is_none());
    }

    #[test]
    fn samesite_none_only_allows_samesite_none_secure_cookie() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::SameSiteNoneOnly);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        let h = headers(&["uid=x; Path=/; SameSite=None; Secure"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert_eq!(
            jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite).as_deref(),
            Some("uid=x")
        );
    }

    #[test]
    fn samesite_none_without_secure_is_blocked() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::SameSiteNoneOnly);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        // SameSite=None but missing Secure flag.
        let h = headers(&["uid=x; Path=/; SameSite=None"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert!(jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite).is_none());
    }

    // ── SameSite attribute enforcement ────────────────────────────────────────

    #[test]
    fn strict_cookie_only_sent_same_site() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["s=1; Path=/; SameSite=Strict"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("s=1"),
            "Strict cookie must be sent on same-site requests"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation).is_none(),
            "Strict cookie must not be sent on cross-site navigation"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite).is_none(),
            "Strict cookie must not be sent on cross-site subrequest"
        );
    }

    #[test]
    fn lax_cookie_sent_same_site_and_navigation_only() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["l=1; Path=/; SameSite=Lax"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("l=1"),
            "Lax cookie must be sent on same-site requests"
        );
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation).as_deref(),
            Some("l=1"),
            "Lax cookie must be sent on cross-site top-level navigation"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite).is_none(),
            "Lax cookie must not be sent on cross-site subrequest"
        );
    }

    #[test]
    fn no_samesite_attribute_defaults_to_lax() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        // No SameSite attribute — RFC 6265bis defaults to Lax.
        jar.store_response_cookies(&req, &headers(&["n=1; Path=/"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("n=1")
        );
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation).as_deref(),
            Some("n=1")
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite).is_none(),
            "Cookie with no SameSite must be blocked on cross-site subrequest (Lax default)"
        );
    }

    #[test]
    fn samesite_none_secure_sent_in_all_contexts() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["x=1; Path=/; SameSite=None; Secure"]), None);

        for ctx in [SameSiteContext::SameSite, SameSiteContext::CrossSiteNavigation, SameSiteContext::CrossSite] {
            assert_eq!(
                jar.get_request_cookies(&req, None, ctx).as_deref(),
                Some("x=1"),
                "SameSite=None; Secure must be sent in all contexts"
            );
        }
    }

    // ── Expiry / Max-Age ──────────────────────────────────────────────────────

    #[test]
    fn expired_cookie_not_sent() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        // Store a cookie with an expiry in the past (Unix epoch = 1970).
        let h = headers(&["old=1; Path=/; Max-Age=-1"]);
        jar.store_response_cookies(&req, &h, None);

        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "expired cookie must not be sent"
        );
    }

    #[test]
    fn max_age_zero_deletes_existing_cookie() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["tok=abc; Path=/"]), None);
        assert!(jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_some());

        // Max-Age=0 means delete.
        jar.store_response_cookies(&req, &headers(&["tok=abc; Path=/; Max-Age=0"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "Max-Age=0 must delete the cookie"
        );
    }

    #[test]
    fn future_max_age_cookie_is_sent() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        let h = headers(&["sess=x; Path=/; Max-Age=3600"]);
        jar.store_response_cookies(&req, &h, None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("sess=x"),
            "cookie with future Max-Age must be sent"
        );
    }

    #[test]
    fn session_cookie_is_sent() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        let h = headers(&["s=1; Path=/"]);
        jar.store_response_cookies(&req, &h, None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("s=1"),
            "session cookie (no expires) must always be sent"
        );
    }

    #[test]
    fn max_age_takes_precedence_over_expires() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        // Expires is in the past, but Max-Age is 1 hour from now — Max-Age wins.
        let h = headers(&["t=1; Path=/; Expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=3600"]);
        jar.store_response_cookies(&req, &h, None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).as_deref(),
            Some("t=1"),
            "Max-Age must override a past Expires date"
        );
    }

    #[test]
    fn purge_expired_removes_stale_cookies() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["good=1; Path=/; Max-Age=3600"]), None);
        // Manually insert an already-expired cookie to simulate time passing.
        jar.entries
            .entry(req.origin().ascii_serialization())
            .or_default()
            .push(crate::engine::cookies::Cookie {
                name: "stale".into(),
                value: "old".into(),
                path: Some("/".into()),
                domain: None,
                secure: false,
                expires: Some(1), // 1970
                same_site: None,
                http_only: false,
            });

        jar.purge_expired();

        let cookies = jar.get_request_cookies(&req, None, SameSiteContext::SameSite).unwrap_or_default();
        assert!(cookies.contains("good=1"), "non-expired cookie must survive purge");
        assert!(!cookies.contains("stale=old"), "expired cookie must be removed by purge");
    }

    #[test]
    fn samesite_none_without_secure_blocked_everywhere() {
        let mut jar = DefaultCookieJar::new();
        let req = url("http://example.com/"); // HTTP, not HTTPS
        jar.store_response_cookies(&req, &headers(&["x=1; Path=/; SameSite=None"]), None);

        for ctx in [SameSiteContext::SameSite, SameSiteContext::CrossSiteNavigation, SameSiteContext::CrossSite] {
            assert!(
                jar.get_request_cookies(&req, None, ctx).is_none(),
                "SameSite=None without Secure must be blocked everywhere"
            );
        }
    }
}
