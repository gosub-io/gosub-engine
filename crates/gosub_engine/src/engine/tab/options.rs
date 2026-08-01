//! Tab configuration: a tab inherits settings from its parent [`Zone`](crate::zone)
//! ([`TabDefaults`]) unless overridden per tab via [`TabOverrides`].

use crate::cookies::CookieJarHandle;
use crate::storage::{PartitionKey, StorageService};
use gosub_render_pipeline::render::Viewport;
use std::sync::Arc;

/// Default parameters for a newly created tab.
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
/// feature - overrides without a consumer don't exist.
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
