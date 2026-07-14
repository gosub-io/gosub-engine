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
use crate::storage::{PartitionKey, StorageService};
use gosub_render_pipeline::render::Viewport;
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
/// By providing overrides, you can control services, partitioning, and identity
/// for a single tab. Fields are added here as the engine grows the corresponding
/// feature — overrides without a consumer don't exist.
///
/// # Example
/// ```no_run
/// use gosub_engine::tab::{TabOverrides, TabCookieJar};
///
/// let overrides = TabOverrides {
///     cookie_jar: TabCookieJar::Ephemeral,           // fresh cookie jar for this tab
///     accept_language: Some("nl-NL,nl;q=0.9".into()), // per-tab Accept-Language
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

    // --- Identity ---
    /// Per-tab `Accept-Language` header override. `None` = inherit the zone's
    /// [`ZoneConfig::accept_languages`](crate::zone::ZoneConfig::accept_languages).
    pub accept_language: Option<String>,
}

/// Policy for selecting a tab's cookie jar.
///
/// Tabs can either inherit their zone’s cookie jar, create a temporary one,
/// or use a fully custom [`CookieJarHandle`].
#[derive(Clone, Debug, Default)]
pub enum TabCookieJar {
    /// Use the zone’s cookie jar (default).
    #[default]
    Inherit,

    /// Fresh ephemeral cookie jar, dropped when the tab is closed.
    Ephemeral,

    /// Custom cookie jar provided by the caller.
    Custom(CookieJarHandle),
}

/// Policy for selecting a tab's storage scope.
///
/// Tabs can either inherit their zone’s [`StorageService`], create an
/// ephemeral in-memory service, or use a custom one.
#[derive(Clone, Debug, Default)]
pub enum TabStorageScope {
    /// Use the zone’s storage service (default).
    #[default]
    Inherit,

    /// Ephemeral in-memory Local/Session storage, isolated per tab.
    Ephemeral,

    /// Custom storage service provided by the caller.
    Custom(Arc<StorageService>),
}
