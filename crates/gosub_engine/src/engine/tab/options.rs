//! Tab configuration, behind [`TabBuilder`](crate::TabBuilder): what a tab does first
//! ([`TabDefaults`]) and how it is isolated from its zone ([`TabOverrides`]).

use crate::cookies::CookieJarHandle;
use crate::storage::{PartitionKey, StorageService};
use gosub_render_pipeline::render::Viewport;
use std::sync::Arc;

/// Initial conditions for a newly created tab.
#[derive(Clone, Debug, Default)]
pub struct TabDefaults {
    /// Initial URL to navigate to.
    pub url: Option<String>,

    /// Initial title, used if no document title is available.
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
/// Set through [`TabBuilder`](crate::TabBuilder):
/// ```no_run
/// # use gosub_engine::{tab::TabCookieJar, zone::Zone};
/// # async fn f(zone: &mut Zone) -> Result<(), gosub_engine::EngineError> {
/// let tab = zone
///     .tab_builder()
///     .cookie_jar(TabCookieJar::Ephemeral)   // fresh cookie jar for this tab
///     .accept_language("nl-NL,nl;q=0.9")     // per-tab Accept-Language
///     .create()
///     .await?;
/// # Ok(()) }
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
