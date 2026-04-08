//! Tab configuration options.
//!
//! This module defines the configuration types used when creating or customizing a tab.
//! A tab can inherit settings from its parent [`Zone`](crate::zone), or override them
//! via [`TabOverrides`].
//!
//! There are two primary entry points:
//! - [`TabDefaults`] — baseline values for new tabs (initial URL, title, viewport).
//! - [`TabOverrides`] — per-tab overrides for services, identity, content, UI, and persistence.
//!
//! Together, these structures provide fine-grained control over how each tab behaves
//! within the Gosub engine.

use crate::cookies::CookieJarHandle;
use crate::render::Viewport;
use crate::storage::{PartitionKey, StorageService};
use std::sync::Arc;

/// Default parameters for a newly created tab.
///
/// These values are *initial conditions* for the tab. They are optional and
/// usually provided by the caller when creating a tab.
///
/// - [`url`](Self::url): initial URL to load
/// - [`title`](Self::title): optional title (used if no document title is available)
/// - [`viewport`](Self::viewport): initial viewport size
#[derive(Clone, Debug, Default)]
pub struct TabDefaults {
    /// Initial URL to navigate to.
    pub url: Option<String>,

    /// Optional initial title for the tab.
    pub title: Option<String>,

    /// Initial viewport configuration (width, height, scroll offset).
    pub viewport: Option<Viewport>,
}

/// Per-tab overrides for configuration.
///
/// A tab normally inherits its settings from the surrounding [`Zone`](crate::zone).
/// By providing overrides, you can control services, identity, content flags,
/// UI properties, and persistence for a single tab.
///
/// # Example
/// ```no_run
/// use gosub_engine::tab::{TabOverrides, TabCookieJar};
///
/// let overrides = TabOverrides {
///     cookie_jar: TabCookieJar::Ephemeral, // fresh cookie jar for this tab
///     js_enabled: Some(false),             // disable JavaScript
///     ..Default::default()
/// };
/// ```
#[derive(Clone, Debug, Default)]
pub struct TabOverrides {
    // --- Services & partitioning ---
    /// Storage partition key. `None` = inherit zone policy.
    pub partition_key: Option<PartitionKey>,

    /// Cookie jar selection (inherit, ephemeral, or custom).
    pub cookie_jar: TabCookieJar, // Default::Inherit

    /// Storage scope (inherit zone service, ephemeral, or custom).
    pub storage_scope: TabStorageScope, // Default::Inherit

    /// Cache mode (inherit, default, bypass, or ephemeral).
    pub cache_mode: TabCacheMode, // Default::Inherit

    // --- Identity ---
    /// Per-tab User-Agent override.
    pub user_agent: Option<String>,

    /// Per-tab `Accept-Language` override.
    pub accept_language: Option<Vec<String>>,

    // --- Content flags ---
    /// Whether JavaScript is enabled for this tab.
    pub js_enabled: Option<bool>,

    /// Whether images are enabled for this tab.
    pub images_enabled: Option<bool>,

    // --- UI/Render ---
    /// Zoom factor override (e.g. `1.0` = 100%).
    pub zoom: Option<f32>,

    // --- Persistence ---
    /// Whether history persistence is enabled for this tab.
    pub persist_history: Option<bool>,

    /// Whether downloads are persisted for this tab.
    pub persist_downloads: Option<bool>,
}

/// Policy for selecting a tab's cookie jar.
///
/// Tabs can either inherit their zone’s cookie jar, create a temporary one,
/// or use a fully custom [`CookieJarHandle`].
#[derive(Clone, Debug)]
pub enum TabCookieJar {
    /// Use the zone’s cookie jar (default).
    Inherit,

    /// Fresh ephemeral cookie jar, dropped when the tab is closed.
    Ephemeral,

    /// Custom cookie jar provided by the caller.
    Custom(CookieJarHandle),
}

impl Default for TabCookieJar {
    fn default() -> Self {
        Self::Inherit
    }
}

/// Policy for selecting a tab's storage scope.
///
/// Tabs can either inherit their zone’s [`StorageService`], create an
/// ephemeral in-memory service, or use a custom one.
#[derive(Clone, Debug)]
pub enum TabStorageScope {
    /// Use the zone’s storage service (default).
    Inherit,

    /// Ephemeral in-memory Local/Session storage, isolated per tab.
    Ephemeral,

    /// Custom storage service provided by the caller.
    Custom(Arc<StorageService>),
}

impl Default for TabStorageScope {
    fn default() -> Self {
        Self::Inherit
    }
}

/// Cache policy for a tab.
///
/// Controls whether the tab uses the inherited cache policy, the default
/// engine policy, bypasses the cache entirely, or uses an ephemeral
/// per-tab cache.
#[derive(Clone, Debug)]
pub enum TabCacheMode {
    /// Use the zone/engine cache policy (default).
    Inherit,

    /// Use the engine’s default cache policy.
    Default,

    /// Bypass the cache entirely.
    Bypass,

    /// Use an ephemeral per-tab cache.
    Ephemeral,
}

impl Default for TabCacheMode {
    fn default() -> Self {
        Self::Inherit
    }
}
