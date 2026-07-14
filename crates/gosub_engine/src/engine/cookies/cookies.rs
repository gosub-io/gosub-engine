//! Cookie core types.
//!
//! This module defines the **type-erased handles** used throughout the engine
//! and the serializable [`Cookie`] data structure.
//!
//! # Concurrency model
//! - [`CookieJarHandle`] is `Arc<RwLock<dyn CookieJar + Send + Sync>>`.
//!   - Callers take a **read lock** for non-mutating operations and a **write lock**
//!     for mutating operations on the underlying jar.
//! - [`CookieStoreHandle`] is `Arc<dyn CookieStore + Send + Sync>`.
//!   - Stores are expected to manage their **own internal synchronization** (e.g. via
//!     `parking_lot`, `Mutex`, connection pools, etc.). The trait methods take `&self`.
//!
//! # Typical usage
//! ```ignore,no_run
//! // Acquire cookies for a request.
//! // Pass the tab's current page URL as `top_level` so the jar can apply
//! // the configured ThirdPartyCookiePolicy for cross-site sub-requests.
//! let jar = zone.cookie_jar(); // -> CookieJarHandle
//! let cookies_header = {
//!     let guard = jar.read();
//!     guard.get_request_cookies(&request_url, Some(&page_url), SameSiteContext::SameSite)
//! };
//!
//! // Store cookies from a response (pass top_level for third-party enforcement).
//! {
//!     let mut guard = jar.write();
//!     guard.store_response_cookies(&request_url, &headers, Some(&page_url));
//! }
//! ```
//!
//! The [`Cookie`] struct is used for persistence/inspection and can be (de)serialized
//! via `serde` to JSON or other formats.
//!
//! ```rust,no_run
//! use gosub_engine::cookies::Cookie;
//!
//! let c = Cookie {
//!     name: "session".into(),
//!     value: "abc123".into(),
//!     path: Some("/".into()),
//!     domain: Some("example.com".into()),
//!     secure: true,
//!     expires: Some(1735689599), // 2025-12-31T23:59:59 UTC as Unix timestamp
//!     same_site: Some("Lax".into()),                 // "Strict" | "Lax" | "None"
//!     http_only: true,
//!     created_at: 0,
//! };
//! ```

use crate::cookies::DefaultCookieJar;
use crate::engine::cookies::store::CookieStore;
use crate::engine::cookies::CookieJar;
use crate::zone::ZoneId;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

/// A handle to a cookie jar trait.
///
/// This is a reference-counted, read/write-locked pointer to a type-erased
/// [`CookieJar`]. Obtain a **read lock** for queries and a **write lock** for
/// mutations.
///
/// ### Example
/// ```ignore,no_run
/// let jar: CookieJarHandle = zone.cookie_jar();
/// {
///     let cookies = jar.read().get_request_cookies(&url, Some(&top_level_url), SameSiteContext::SameSite);
/// }
/// {
///     let mut guard = jar.write();
///     guard.clear();
/// }
/// ```
#[derive(Clone, Debug)]
pub struct CookieJarHandle(Arc<RwLock<Box<dyn CookieJar + Send + Sync>>>);

impl Debug for dyn CookieJar + Send + Sync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieJar {{ ... }}")
    }
}

impl CookieJarHandle {
    /// Pointer equality: are these two handles backed by the same Arc?
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        Arc::ptr_eq(&this.0, &other.0)
    }
}

impl PartialEq for CookieJarHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for CookieJarHandle {}

impl CookieJarHandle {
    pub fn new<T>(jar: T) -> Self
    where
        T: CookieJar + Send + Sync + 'static,
    {
        Self(Arc::new(RwLock::new(Box::new(jar))))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Box<dyn CookieJar + Send + Sync>> {
        self.0.read()
    }
    pub fn write(&self) -> RwLockWriteGuard<'_, Box<dyn CookieJar + Send + Sync>> {
        self.0.write()
    }
}

impl Deref for CookieJarHandle {
    type Target = RwLock<Box<dyn CookieJar + Send + Sync>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Box<dyn CookieJar + Send + Sync>> for CookieJarHandle {
    fn from(jar: Box<dyn CookieJar + Send + Sync>) -> Self {
        Self(Arc::new(RwLock::new(jar)))
    }
}

impl<T> From<T> for CookieJarHandle
where
    T: CookieJar + Send + Sync + 'static,
{
    fn from(jar: T) -> Self {
        Self::new(jar)
    }
}

/// A handle to a cookie store trait.
///
/// This is a reference-counted pointer to a type-erased [`CookieStore`].
/// Store implementations must be **`Send + Sync` and internally synchronized**,
/// since callers hold only `&self` when invoking trait methods.
///
/// Typical use is at **build/initialization time** to mint a per-zone jar.
#[derive(Clone)]
pub struct CookieStoreHandle(Arc<dyn CookieStore + Send + Sync>);

impl<T> From<Arc<T>> for CookieStoreHandle
where
    T: CookieStore + Send + Sync + 'static,
{
    fn from(a: Arc<T>) -> Self {
        Self(a)
    }
}

impl Debug for CookieStoreHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieStore {{ ... }}")
    }
}

impl CookieStoreHandle {
    pub fn persist_zone_from_snapshot(&self, zone: ZoneId, snap: &DefaultCookieJar) {
        self.0.persist_zone_from_snapshot(zone, snap);
    }
    pub fn persist_all(&self) {
        self.0.persist_all();
    }
    pub fn release_zone(&self, zone: ZoneId) {
        self.0.release_zone(zone);
    }
    /// Deletes the zone's persisted cookie data (e.g. "clear cookies" / profile removal).
    /// For closing a zone while keeping its data, use [`Self::release_zone`] instead.
    pub fn remove_zone(&self, zone: ZoneId) {
        self.0.remove_zone(zone);
    }
    pub fn jar_for(&self, zone: ZoneId) -> Option<CookieJarHandle> {
        self.0.jar_for(zone)
    }
}

/// A cookie as stored/serialized by the engine.
///
/// This structure captures the essential attributes of an HTTP cookie and
/// is suitable for persistence (e.g., JSON, SQLite) via `serde`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Cookie {
    /// Cookie name (case-sensitive).
    pub name: String,

    /// Raw cookie value (not URL-decoded).
    pub value: String,

    /// Path scoping (e.g., `"/"`). If `None`, path-matching follows RFC defaults.
    pub path: Option<String>,

    /// Domain scoping (host-only if `None`). When present, should be a registrable domain
    /// or subdomain (e.g., `"example.com"`).
    pub domain: Option<String>,

    /// If `true`, cookie is sent only over HTTPS.
    pub secure: bool,

    /// Expiration time as **Unix timestamp** (seconds since 1970-01-01T00:00:00Z).
    ///
    /// Computed at receive time from `Max-Age` (preferred) or the `Expires` date header.
    /// `None` means a session cookie that is not persisted across browser restarts.
    pub expires: Option<i64>,

    /// SameSite policy (`"Strict"`, `"Lax"`, or `"None"`).
    ///
    /// `Option::None` means the attribute was absent; the jar then applies the
    /// modern default of **Lax**. The string value `"None"` (cross-site allowed)
    /// additionally requires `secure=true`.
    /// Consider modeling as an enum in the future.
    pub same_site: Option<String>,

    /// If `true`, cookie is blocked from access by client-side scripts (`document.cookie`).
    pub http_only: bool,

    /// Creation time as Unix timestamp in **milliseconds**.
    ///
    /// Set once when the cookie is first stored; preserved on subsequent updates to
    /// the same (name, domain, path) triple so that creation order survives overwrites.
    /// Used to break ties when two cookies have equal-length paths — earlier cookies
    /// are sent first (RFC 6265bis §5.5).
    ///
    /// Defaults to `0` so that cookies persisted before this field was added still
    /// deserialise correctly.
    #[serde(default)]
    pub created_at: i64,
}
