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
//! - The attributes `Expires`, `Max-Age`, `Path`, `Domain`, `Secure`,
//!   `HttpOnly`, and `SameSite` are parsed and enforced; expired cookies are
//!   filtered on read and can be removed via [`CookieJar::purge_expired`].
//!   Priorities, size limits, and eviction policies are not (yet) implemented.
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
use psl::Psl as _;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use url::Url;

/// Returns `true` if `request_host` may set a cookie scoped to `domain`.
///
/// Enforces two RFC 6265 rules:
/// 1. `domain` must be a registrable-domain suffix of `request_host` (§5.3 step 6).
/// 2. `domain` must not itself be a known public suffix / eTLD such as `"com"` or
///    `"co.uk"` (RFC 6265bis §5.4).  Unknown TLDs (e.g. `localhost`, intranet names)
///    are allowed because the PSL may not list them.
fn is_valid_cookie_domain(request_host: &str, domain: &str) -> bool {
    // RFC 4343: domain comparisons are case-insensitive.
    let req = request_host.cow_to_ascii_lowercase();
    let dom = domain.cow_to_ascii_lowercase();

    // Rule 1: domain must be a registrable-domain suffix of the request host.
    if req != dom && !req.ends_with(&format!(".{dom}")) {
        return false;
    }

    // Rule 2: reject bare public suffixes (eTLDs) using the compiled Mozilla PSL.
    //
    // `psl::List.domain()` returns None when the input has no registrable portion,
    // i.e. the input itself is a bare eTLD ("com", "co.uk", "github.io").
    // We combine this with `suffix().is_known()` to distinguish known eTLDs from
    // labels that are simply absent from the PSL ("localhost", intranet names) —
    // the latter should be allowed even though domain() also returns None for them.
    if psl::List.domain(domain.as_bytes()).is_none()
        && psl::List.suffix(domain.as_bytes()).is_some_and(|s| s.is_known())
    {
        return false;
    }

    true
}

/// Parse an HTTP date string into a Unix timestamp.
///
/// Handles three formats in order of preference:
/// 1. RFC 2822 with numeric offset (`+0000`) — e.g. from well-behaved clients.
/// 2. RFC 1123 / HTTP-date (`GMT` timezone) — the dominant real-world format
///    per RFC 7231 §7.1.1.1: `"Fri, 07 Aug 2007 08:04:19 GMT"`.
/// 3. RFC 850 / obsolete format (`"Friday, 07-Aug-07 08:04:19 GMT"`).
///
/// Returns `None` if none of the formats match, which causes the cookie to be
/// treated as a session cookie rather than silently accepting a bad expiry.
fn parse_http_date(s: &str) -> Option<i64> {
    use chrono::NaiveDateTime;

    let s = s.trim();

    // 1. Strict RFC 2822 (numeric timezone offset)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
        return Some(dt.timestamp());
    }

    // 2. RFC 1123 "Fri, 07 Aug 2007 08:04:19 GMT"
    //    Strip the timezone suffix, then strip the optional "Day, " prefix before
    //    parsing — chrono's %a can be locale-sensitive; avoiding it is more robust.
    let bare = s.strip_suffix(" GMT").or_else(|| s.strip_suffix("GMT")).unwrap_or(s);
    // Strip "Weekday, " prefix if present.
    let bare = bare.find(", ").map(|i| &bare[i + 2..]).unwrap_or(bare);
    if let Ok(ndt) = NaiveDateTime::parse_from_str(bare, "%d %b %Y %H:%M:%S") {
        return Some(ndt.and_utc().timestamp());
    }

    // 3. RFC 850 "Friday, 07-Aug-07 08:04:19 GMT" (already stripped prefix above)
    if let Ok(ndt) = NaiveDateTime::parse_from_str(bare, "%d-%b-%y %H:%M:%S") {
        return Some(ndt.and_utc().timestamp());
    }

    None
}

/// Returns `true` when two hostnames share the same registrable domain (eTLD+1).
///
/// Uses the compile-time embedded Mozilla Public Suffix List (`psl` crate) for
/// accurate comparison. Falls back to exact hostname equality for IP addresses,
/// `localhost`, and other labels not present in the PSL.
fn same_site(host_a: &str, host_b: &str) -> bool {
    let registrable = |host: &str| -> Option<String> {
        let d = psl::List.domain(host.as_bytes())?;
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
///   (parsed into a unix timestamp), `SameSite` (`Strict`/`Lax`/`None`, case-insensitive),
///   `Secure`, `HttpOnly`.
/// - If `Path` is absent, a default path is derived from the request URL.
/// - Expired cookies are filtered out on read; [`Self::purge_expired`] removes them
///   from the jar.
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

        let request_host = url.host_str().unwrap_or_default();
        let origin = url.origin().ascii_serialization();
        let default_path = url
            .path()
            .rsplit_once('/')
            .map_or("/", |(a, _)| if a.is_empty() { "/" } else { a });

        let bucket = self.entries.entry(origin).or_default();

        for header in headers.get_all("set-cookie") {
            // Use from_utf8 (not to_str) so that non-ASCII cookie values (e.g.
            // UTF-8 encoded characters) are accepted rather than silently dropped.
            if let Ok(header_str) = std::str::from_utf8(header.as_bytes()) {
                if let Some((name, rest)) = header_str.split_once('=') {
                    let cookie_name = name.trim();
                    // RFC 6265 §5.2: the name-value-pair is the portion before the
                    // first ';'. If the extracted name is empty or contains ';'
                    // (which means the ';' appeared before '='), the header is invalid.
                    if cookie_name.is_empty() || cookie_name.contains(';') {
                        continue;
                    }
                    let mut cookie = Cookie {
                        name: cookie_name.to_string(),
                        value: String::new(),
                        path: None,
                        domain: None,
                        secure: false,
                        expires: None,
                        same_site: None,
                        http_only: false,
                        created_at: 0, // set in the dedup/insert block below
                    };

                    // Collected during attribute parsing; resolved after the loop.
                    let mut max_age: Option<i64> = None;
                    let mut expires_str: Option<String> = None;
                    let mut domain_rejected = false;
                    // Separate flag so that an empty value "" doesn't cause the
                    // next attribute to be mistakenly treated as the value.
                    let mut value_parsed = false;

                    for part in rest.split(';') {
                        let part = part.trim();
                        if !value_parsed {
                            cookie.value = part.to_string();
                            value_parsed = true;
                            continue;
                        }

                        if let Some((k, v)) = part.split_once('=') {
                            // Trim the attribute name: "Secure =" is equivalent to "Secure".
                            match k.trim().cow_to_ascii_lowercase().as_ref() {
                                "path" => {
                                    // Strip surrounding double-quotes — browsers tolerate
                                    // quoted path values and the semicolon delimiter inside
                                    // them splits the raw header, leaving a stray leading '"'.
                                    let p = v.trim().trim_matches('"');
                                    if p.starts_with('/') {
                                        cookie.path = Some(p.to_string());
                                    } else {
                                        cookie.path = Some(v.trim().to_string());
                                    }
                                }
                                "domain" => {
                                    // Strip leading dot, then normalise to lowercase (RFC 4343).
                                    let d = v.trim().trim_start_matches('.').cow_to_ascii_lowercase();
                                    if is_valid_cookie_domain(request_host, &d) {
                                        cookie.domain = Some(d.into_owned());
                                    } else {
                                        domain_rejected = true;
                                    }
                                }
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
                                // "Secure=anything" and "HttpOnly=anything" are treated
                                // as the bare flag form (browsers are lenient here).
                                "secure" => cookie.secure = true,
                                "httponly" => cookie.http_only = true,
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

                    // Drop the cookie if the Domain attribute failed validation (RFC 6265 §5.3).
                    if domain_rejected {
                        continue;
                    }

                    // Reject Secure cookies over plain HTTP (RFC 6265bis §4.1.2.1).
                    // Only HTTPS responses may set cookies with the Secure attribute.
                    if cookie.secure && url.scheme() != "https" {
                        continue;
                    }

                    // Enforce cookie name prefixes (RFC 6265bis §4.1.3).
                    //
                    // __Secure-: cookie must have the Secure attribute.
                    if cookie.name.starts_with("__Secure-") && !cookie.secure {
                        continue;
                    }
                    // __Host-: cookie must have Secure, no Domain attribute, and Path=/.
                    if cookie.name.starts_with("__Host-")
                        && (!cookie.secure || cookie.domain.is_some() || cookie.path.as_deref() != Some("/"))
                    {
                        continue;
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

                    // Cookies are unique by (name, domain, path) — RFC 6265bis §5.6.
                    // On update, preserve the original creation time so path-based ordering
                    // (RFC 6265bis §5.5) reflects when the cookie was *first* set.
                    if let Some(existing) = bucket
                        .iter_mut()
                        .find(|c| c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
                    {
                        let original_created_at = existing.created_at;
                        *existing = cookie;
                        existing.created_at = original_created_at;
                    } else {
                        cookie.created_at = Utc::now().timestamp_millis();
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
        // RFC 4343: domain comparison is case-insensitive.
        let host_lower = url.host_str().unwrap_or_default().cow_to_ascii_lowercase();
        let path = url.path();
        let is_https = url.scheme() == "https";

        let now = Utc::now().timestamp();

        // Scan ALL origin buckets:
        // - Domain cookies (Some(domain)): send if request host matches the domain attribute,
        //   regardless of which origin set the cookie.
        // - Host-only cookies (None domain): send only from the exact same origin (RFC 6265 §5.3,
        //   "host-only-flag").
        let mut matching: Vec<&Cookie> = self
            .entries
            .iter()
            .flat_map(|(bucket_origin, cookies)| cookies.iter().map(move |c| (bucket_origin.as_str(), c)))
            .filter(|(_, cookie)| {
                // Drop expired cookies (session cookies have expires == None).
                cookie.expires.is_none_or(|exp| exp > now)
            })
            .filter(|(_, cookie)| {
                // Third-party policy: SameSiteNoneOnly allows only None+Secure cross-site.
                if is_third_party && self.third_party_policy == ThirdPartyCookiePolicy::SameSiteNoneOnly {
                    return matches!(cookie.same_site.as_deref(), Some("None")) && cookie.secure;
                }
                true
            })
            .filter(|(_, cookie)| {
                // SameSite attribute enforcement (RFC 6265bis).
                // Cookies with no SameSite attribute default to Lax behavior.
                match cookie.same_site.as_deref() {
                    Some("Strict") => samesite == SameSiteContext::SameSite,
                    Some("None") => cookie.secure,
                    // Lax (explicit) or absent (implicit Lax default)
                    _ => matches!(
                        samesite,
                        SameSiteContext::SameSite | SameSiteContext::CrossSiteNavigation
                    ),
                }
            })
            .filter(|(bucket_origin, cookie)| {
                match &cookie.domain {
                    Some(domain) => {
                        // Domain cookie: case-insensitive match against request host.
                        let d = domain.cow_to_ascii_lowercase();
                        host_lower == d || host_lower.ends_with(&format!(".{d}"))
                    }
                    None => {
                        // Host-only cookie: must originate from the exact same origin.
                        *bucket_origin == origin.as_str()
                    }
                }
            })
            .filter(|(_, cookie)| match &cookie.path {
                // RFC 6265 §5.1.4 path-match.
                Some(cookie_path) => {
                    path == cookie_path
                        || (path.starts_with(cookie_path.as_str())
                            && (cookie_path.ends_with('/') || path[cookie_path.len()..].starts_with('/')))
                }
                None => true,
            })
            .filter(|(_, cookie)| !cookie.secure || is_https)
            .map(|(_, c)| c)
            .collect::<Vec<_>>();

        // RFC 6265bis §5.5: cookies with longer paths are sent first;
        // ties broken by creation time ascending (earlier = higher priority).
        matching.sort_by(|a, b| {
            let len_a = a.path.as_deref().map_or(0, str::len);
            let len_b = b.path.as_deref().map_or(0, str::len);
            len_b.cmp(&len_a).then_with(|| a.created_at.cmp(&b.created_at))
        });

        let header = matching
            .iter()
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
            cookies.retain(|c| c.expires.is_none_or(|exp| exp > now));
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
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
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
            jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSiteNavigation)
                .as_deref(),
            Some("uid=x")
        );
        // The same cookie is still blocked on a cross-site subrequest (Lax default).
        assert!(jar
            .get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite)
            .is_none());
    }

    // ── ThirdPartyCookiePolicy::Block ─────────────────────────────────────────

    #[test]
    fn block_policy_prevents_third_party_storage() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert!(jar
            .get_request_cookies(&resource, None, SameSiteContext::SameSite)
            .is_none());
    }

    #[test]
    fn block_policy_prevents_third_party_sending() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        // Store as first-party (no top_level), then try to send as third-party.
        let resource = url("https://tracker.com/pixel");
        let h = headers(&["uid=x; Path=/"]);
        jar.store_response_cookies(&resource, &h, None);

        let top = url("https://example.com/");
        assert!(jar
            .get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite)
            .is_none());
    }

    #[test]
    fn block_policy_allows_first_party() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::Block);
        let req = url("https://example.com/page");
        let top = url("https://example.com/");
        let h = headers(&["id=1; Path=/"]);
        jar.store_response_cookies(&req, &h, Some(&top));

        assert_eq!(
            jar.get_request_cookies(&req, Some(&top), SameSiteContext::SameSite)
                .as_deref(),
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

        assert!(jar
            .get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite)
            .is_none());
    }

    #[test]
    fn samesite_none_only_allows_samesite_none_secure_cookie() {
        let mut jar = DefaultCookieJar::new().with_policy(ThirdPartyCookiePolicy::SameSiteNoneOnly);
        let resource = url("https://tracker.com/pixel");
        let top = url("https://example.com/");
        let h = headers(&["uid=x; Path=/; SameSite=None; Secure"]);
        jar.store_response_cookies(&resource, &h, Some(&top));

        assert_eq!(
            jar.get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite)
                .as_deref(),
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

        assert!(jar
            .get_request_cookies(&resource, Some(&top), SameSiteContext::CrossSite)
            .is_none());
    }

    // ── SameSite attribute enforcement ────────────────────────────────────────

    #[test]
    fn strict_cookie_only_sent_same_site() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["s=1; Path=/; SameSite=Strict"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("s=1"),
            "Strict cookie must be sent on same-site requests"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation)
                .is_none(),
            "Strict cookie must not be sent on cross-site navigation"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite)
                .is_none(),
            "Strict cookie must not be sent on cross-site subrequest"
        );
    }

    #[test]
    fn lax_cookie_sent_same_site_and_navigation_only() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["l=1; Path=/; SameSite=Lax"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("l=1"),
            "Lax cookie must be sent on same-site requests"
        );
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation)
                .as_deref(),
            Some("l=1"),
            "Lax cookie must be sent on cross-site top-level navigation"
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite)
                .is_none(),
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
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("n=1")
        );
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSiteNavigation)
                .as_deref(),
            Some("n=1")
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::CrossSite)
                .is_none(),
            "Cookie with no SameSite must be blocked on cross-site subrequest (Lax default)"
        );
    }

    #[test]
    fn samesite_none_secure_sent_in_all_contexts() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["x=1; Path=/; SameSite=None; Secure"]), None);

        for ctx in [
            SameSiteContext::SameSite,
            SameSiteContext::CrossSiteNavigation,
            SameSiteContext::CrossSite,
        ] {
            assert_eq!(
                jar.get_request_cookies(&req, None, ctx).as_deref(),
                Some("x=1"),
                "SameSite=None; Secure must be sent in all contexts"
            );
        }
    }

    // ── Domain validation ─────────────────────────────────────────────────────

    #[test]
    fn psl_domain_behavior() {
        let reg = |h: &str| -> Option<String> {
            psl::List
                .domain(h.as_bytes())
                .and_then(|d| std::str::from_utf8(d.as_bytes()).ok().map(str::to_owned))
        };
        // Known registrable domains — psl::List must return the eTLD+1.
        assert_eq!(reg("a.com"), Some("a.com".into()));
        assert_eq!(reg("a.co.uk"), Some("a.co.uk".into())); // co.uk IS a PSL entry
        assert_eq!(reg("a.github.io"), Some("a.github.io".into())); // github.io is a private PSL entry
        assert_eq!(reg("example.com"), Some("example.com".into()));
        // Bare eTLDs — PSL must return None (no registrable portion above the suffix).
        assert_eq!(reg("com"), None);
        assert_eq!(reg("co.uk"), None);
        assert_eq!(reg("github.io"), None);
        // Unknown labels — PSL must also return None (not in the list).
        assert_eq!(reg("localhost"), None);
    }

    #[test]
    fn valid_domain_suffix_is_accepted() {
        assert!(is_valid_cookie_domain("foo.example.com", "example.com"));
        assert!(is_valid_cookie_domain("example.com", "example.com"));
        assert!(is_valid_cookie_domain("a.b.example.com", "example.com"));
    }

    #[test]
    fn domain_not_a_suffix_is_rejected() {
        assert!(!is_valid_cookie_domain("example.com", "other.com"));
        assert!(!is_valid_cookie_domain("foo.example.com", "bar.example.com"));
        assert!(!is_valid_cookie_domain("example.com", "www.example.com")); // more specific
    }

    #[test]
    fn public_suffix_domain_is_rejected() {
        assert!(!is_valid_cookie_domain("foo.com", "com"));
        assert!(!is_valid_cookie_domain("foo.co.uk", "co.uk"));
        assert!(!is_valid_cookie_domain("foo.github.io", "github.io")); // private PSL entry
    }

    #[test]
    fn localhost_domain_is_allowed() {
        assert!(is_valid_cookie_domain("localhost", "localhost"));
    }

    #[test]
    fn invalid_domain_causes_cookie_to_be_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");

        // Domain is a public suffix — entire cookie must be dropped.
        jar.store_response_cookies(&req, &headers(&["id=1; Path=/; Domain=com"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "cookie with Domain=com must be silently dropped"
        );
    }

    #[test]
    fn cross_domain_cookie_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");

        jar.store_response_cookies(&req, &headers(&["id=1; Path=/; Domain=other.com"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "cookie with Domain=other.com set from example.com must be dropped"
        );
    }

    #[test]
    fn valid_subdomain_cookie_is_stored() {
        let mut jar = DefaultCookieJar::new();
        // Server at foo.example.com sets Domain=example.com — that's valid.
        let req = url("https://foo.example.com/");
        jar.store_response_cookies(&req, &headers(&["s=1; Path=/; Domain=example.com"]), None);

        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("s=1"),
            "cookie with valid Domain superdomain must be stored and returned"
        );
    }

    // ── Secure attribute / HTTP rejection ────────────────────────────────────

    #[test]
    fn secure_cookie_over_http_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("http://example.com/"); // plain HTTP
        jar.store_response_cookies(&req, &headers(&["s=1; Path=/; Secure"]), None);
        // Cookie with Secure flag MUST NOT be stored from an HTTP response.
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "Secure cookie from HTTP must be dropped"
        );
    }

    #[test]
    fn secure_cookie_over_https_is_stored() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["s=1; Path=/; Secure"]), None);
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("s=1")
        );
    }

    #[test]
    fn non_secure_cookie_over_http_is_stored() {
        let mut jar = DefaultCookieJar::new();
        let req = url("http://example.com/");
        jar.store_response_cookies(&req, &headers(&["n=1; Path=/"]), None);
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("n=1"),
            "Plain cookie (no Secure flag) must be accepted over HTTP"
        );
    }

    // ── Cookie name prefix enforcement ────────────────────────────────────────

    #[test]
    fn secure_prefix_accepted_with_secure_flag() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["__Secure-tok=x; Path=/; Secure"]), None);
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("__Secure-tok=x")
        );
    }

    #[test]
    fn secure_prefix_without_secure_flag_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["__Secure-tok=x; Path=/"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "__Secure- without Secure flag must be dropped"
        );
    }

    #[test]
    fn host_prefix_accepted_when_all_conditions_met() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        // Secure + no Domain + Path=/
        jar.store_response_cookies(&req, &headers(&["__Host-sid=1; Path=/; Secure"]), None);
        assert_eq!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
            Some("__Host-sid=1")
        );
    }

    #[test]
    fn host_prefix_without_secure_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(&req, &headers(&["__Host-sid=1; Path=/"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "__Host- without Secure must be dropped"
        );
    }

    #[test]
    fn host_prefix_with_domain_attribute_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/");
        jar.store_response_cookies(
            &req,
            &headers(&["__Host-sid=1; Secure; Path=/; Domain=example.com"]),
            None,
        );
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "__Host- with Domain attribute must be dropped"
        );
    }

    #[test]
    fn host_prefix_with_non_root_path_is_dropped() {
        let mut jar = DefaultCookieJar::new();
        let req = url("https://example.com/app/");
        jar.store_response_cookies(&req, &headers(&["__Host-sid=1; Secure; Path=/app"]), None);
        assert!(
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite).is_none(),
            "__Host- with Path != / must be dropped"
        );
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
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
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
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
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
            jar.get_request_cookies(&req, None, SameSiteContext::SameSite)
                .as_deref(),
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
                created_at: 0,
            });

        jar.purge_expired();

        let cookies = jar
            .get_request_cookies(&req, None, SameSiteContext::SameSite)
            .unwrap_or_default();
        assert!(cookies.contains("good=1"), "non-expired cookie must survive purge");
        assert!(
            !cookies.contains("stale=old"),
            "expired cookie must be removed by purge"
        );
    }

    #[test]
    fn samesite_none_without_secure_blocked_everywhere() {
        let mut jar = DefaultCookieJar::new();
        let req = url("http://example.com/"); // HTTP, not HTTPS
        jar.store_response_cookies(&req, &headers(&["x=1; Path=/; SameSite=None"]), None);

        for ctx in [
            SameSiteContext::SameSite,
            SameSiteContext::CrossSiteNavigation,
            SameSiteContext::CrossSite,
        ] {
            assert!(
                jar.get_request_cookies(&req, None, ctx).is_none(),
                "SameSite=None without Secure must be blocked everywhere"
            );
        }
    }
}
