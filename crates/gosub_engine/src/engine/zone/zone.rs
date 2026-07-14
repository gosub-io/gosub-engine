use crate::cookies::CookieStoreHandle;
use crate::engine::cookies::CookieJarHandle;
use crate::engine::engine::EngineContext;
use crate::engine::events::EngineEvent;
use crate::engine::storage::{StorageService, Subscription};
use crate::engine::tab::TabId;
use crate::engine::types::{EventChannel, IoChannel, TabChannel};
use crate::events::TabCommand;
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::storage::types::PartitionPolicy;
use crate::tab::services::resolve_tab_services;
use crate::tab::{create_tab_and_spawn, TabDefaults, TabHandle, TabOverrides, TabSink};
use crate::util::spawn_named;
use crate::zone::ZoneConfig;
use crate::EngineError;
use gosub_config::Config;
use parking_lot::{Mutex, RwLock};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use uuid::Uuid;

/// A unique identifier for a [`Zone`] within a [`GosubEngine`](crate::GosubEngine).
///
/// Internally, a `ZoneId` wraps a [`Uuid`] to guarantee global uniqueness for
/// each zone created in the engine.
///
/// **Note:** The use of [`Uuid`] is an implementation detail and may change in
/// the future without notice. Always treat `ZoneId` as an opaque handle rather
/// than relying on its internal representation.
///
/// # Purpose
///
/// A `ZoneId` allows the engine and user code to unambiguously reference and
/// operate on a specific [`Zone`], even if multiple zones are created, closed,
/// or restored across sessions.
///
/// # Examples
///
/// Creating a new `ZoneId` manually:
/// ```
/// use gosub_engine::zone::ZoneId;
///
/// let id = ZoneId::new();
/// println!("New zone ID: {:?}", id);
///
/// let uuid = uuid::Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").expect("invalid uuid");
/// let fixed_id = ZoneId::from(uuid);
/// println!("Fixed zone ID: {}", fixed_id);
/// ```
///
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ZoneId(Uuid);

impl Default for ZoneId {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoneId {
    /// Creates a new `ZoneId` with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for ZoneId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<String> for ZoneId {
    fn from(s: String) -> Self {
        let uuid = Uuid::parse_str(&s).unwrap_or_else(|_| Uuid::new_v4());
        Self(uuid)
    }
}

impl Display for ZoneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Services provided to tabs within a zone
#[derive(Clone, Debug)]
pub struct ZoneServices {
    pub storage: Arc<StorageService>,
    /// Cookie store for this zone (if any)
    pub cookie_store: Option<CookieStoreHandle>,
    /// Cookie jar for this zone (if any)
    pub cookie_jar: Option<CookieJarHandle>,
    /// Policy for partitioning storage (cookies, localStorage, etc.)
    pub partition_policy: PartitionPolicy,
}

/// Zone context we can share downwards to tabs
pub struct ZoneContext<C: RenderConfiguration = crate::html::DefaultRenderConfig> {
    /// Zone services (storage, cookies, etc)
    pub(crate) services: ZoneServices,
    /// Subscription for session storage changes
    pub(crate) storage_rx: Subscription,
    /// Flags controlling which data is shared with other zones.
    pub(crate) shared_flags: SharedFlags,
    /// Event channel to send events back to the UI
    pub(crate) event_tx: EventChannel,
    /// Channel to communicate to the network I/O thread
    pub(crate) io_tx: IoChannel,
    /// Map of request references to tab IDs, used to route network events back to the right tab
    pub(crate) request_reference_map: Arc<RwLock<RequestReferenceMap>>,

    /// Compositor sink to use for this zone (concrete, per the module config).
    pub(crate) compositor: Arc<RwLock<C::CompositorSink>>,
    /// Rendering backend to use for this zone (concrete, per the module config).
    pub(crate) render_backend: Arc<C::RenderBackend>,
    /// The engine's shared font system (the config's `FontSystem`), used by the layouter for
    /// measurement and handed to the rasterizer for drawing.
    pub(crate) font_system: Arc<Mutex<C::FontSystem>>,
    /// Per-engine settings store, cloned from the engine context and passed on to each tab.
    pub(crate) config_store: Config,
}

// Things that are shared upwards to the engine
pub struct ZoneSink {
    /// How many tabs has this zone created over its lifetime
    tabs_created: AtomicUsize,
}

/// This is the zone structure, which contains tabs and shared services. It is only known to the engine
/// and can be controlled by the user via the engine API.
pub struct Zone<C: RenderConfiguration = crate::html::DefaultRenderConfig> {
    // Shared context from the engine
    pub engine_context: Arc<EngineContext>,
    // Shared context that is passed down to tabs
    pub context: Arc<ZoneContext<C>>,
    // Shared state that can be read by anyone with a ZoneSink
    pub sink: Arc<ZoneSink>,
    // List of tabs
    tabs: HashMap<TabId, TabInfo>,

    /// ID of the zone
    pub id: ZoneId,
    /// Configuration for the zone (like max tabs allowed)
    config: ZoneConfig,
    /// Title of the zone (ie: Home, Work)
    pub title: String,
    /// Icon of the zone (could be a base64 encoded image)
    pub icon: Vec<u8>,
    /// Description of the zone
    pub description: String,
    /// Tab color (RGBA)
    pub color: [u8; 4],
}

impl<C: RenderConfiguration> Debug for Zone<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zone")
            .field("id", &self.id)
            .field("title", &self.title)
            .field("description", &self.description)
            .field("color", &self.color)
            .field("config", &self.config)
            .field("shared_flags", &self.context.shared_flags)
            .finish()
    }
}

/// Simple structure to hold tab info inside the zone
struct TabInfo {
    /// Command sender for the tab worker, used to ask it to close.
    cmd_tx: TabChannel,
    /// Worker join handle, awaited when the tab is closed.
    join_handle: tokio::task::JoinHandle<()>,
    #[allow(unused)]
    sink: Arc<TabSink>,
}

#[allow(unused)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SharedFlags {
    /// Other zones are allowed to read this autocomplete elements
    pub share_autocomplete: bool,
    /// Other zones are allowed to read bookmarks
    pub share_bookmarks: bool,
    /// Other zones are allowed to read password entries
    pub share_passwords: bool,
    /// Other zones are allowed to read cookies
    pub share_cookiejar: bool,
}

impl<C: RenderConfiguration> Zone<C> {
    /// Creates a new zone with a specific zone ID
    pub fn new_with_id(
        // Unique ID for the zone
        zone_id: ZoneId,
        // Configuration for the zone
        config: ZoneConfig,
        // Services to provide to tabs within this zone
        services: ZoneServices,
        // Event channel to send events back to the UI
        engine_context: Arc<EngineContext>,
        // Render backend / compositor for this engine's config (concrete)
        render_backend: Arc<C::RenderBackend>,
        compositor: Arc<RwLock<C::CompositorSink>>,
        // The engine's shared font system (the config's `FontSystem`)
        font_system: Arc<Mutex<C::FontSystem>>,
    ) -> Result<Self, EngineError> {
        // We generate the color by using the zone id as a seed
        let mut rng = StdRng::seed_from_u64(zone_id.0.as_u64_pair().0);
        let random_color = [
            rng.random::<u8>(),
            rng.random::<u8>(),
            rng.random::<u8>(),
            0xff, // Fully opaque
        ];

        let storage_rx = services.storage.subscribe();
        let event_tx = engine_context.event_tx.clone();
        let io_tx = {
            let guard = engine_context.io_tx.read();
            guard.as_ref().cloned().ok_or(EngineError::IoNotStarted)?
        };
        let request_reference_map = engine_context.request_reference_map.clone();
        let config_store = engine_context.config_store.clone();

        let zone = Self {
            engine_context,
            sink: Arc::new(ZoneSink {
                tabs_created: AtomicUsize::new(0),
            }),
            context: Arc::new(ZoneContext {
                services,
                storage_rx,
                shared_flags: SharedFlags {
                    share_autocomplete: false,
                    share_bookmarks: false,
                    share_passwords: false,
                    share_cookiejar: false,
                },
                event_tx,
                io_tx,
                request_reference_map,
                compositor,
                render_backend,
                font_system,
                config_store,
            }),
            id: zone_id,
            tabs: HashMap::new(),
            title: "Untitled Zone".to_string(),
            icon: vec![],
            description: "".to_string(),
            color: random_color,
            config,
        };

        _ = zone.spawn_storage_events_to_engine();
        Ok(zone)
    }

    /// Creates a new zone with a random ID and the provided configuration
    pub fn new(
        config: ZoneConfig,
        services: ZoneServices,
        engine_context: Arc<EngineContext>,
        render_backend: Arc<C::RenderBackend>,
        compositor: Arc<RwLock<C::CompositorSink>>,
        font_system: Arc<Mutex<C::FontSystem>>,
    ) -> Result<Self, EngineError> {
        Self::new_with_id(
            ZoneId::new(),
            config,
            services,
            engine_context,
            render_backend,
            compositor,
            font_system,
        )
    }

    /// Sets the title of the zone
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// Sets the icon of the zone
    pub fn set_icon(&mut self, icon: Vec<u8>) {
        self.icon = icon;
    }

    /// Sets the description of the zone
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = description.into();
    }

    /// Sets the color of the zone (RGBA)
    pub fn set_color(&mut self, color: [u8; 4]) {
        self.color = color;
    }

    // /// Returns the services available to tabs within this zone
    // pub fn services(&self) -> ZoneServices { self.services.clone() }

    /// Create a new tab in the zone. Will set any initial values provided in `initial`
    /// and apply any overrides to the default services for the tab if any are required.
    pub async fn create_tab(
        &mut self,
        initial: TabDefaults,
        overrides: Option<TabOverrides>,
    ) -> Result<TabHandle, EngineError> {
        if self.tabs.len() >= effective_max_tabs(&self.context.config_store, self.config.max_tabs) {
            return Err(EngineError::TabLimitExceeded);
        }

        let tab_services = resolve_tab_services(
            self.id,
            &self.context.services,
            &self.config,
            &overrides.unwrap_or_default(),
        );

        let (tab_handle, join_handle) =
            create_tab_and_spawn::<C>(self.id, tab_services, self.context.clone()).map_err(EngineError::CreateTab)?;
        self.tabs.insert(
            tab_handle.tab_id,
            TabInfo {
                cmd_tx: tab_handle.cmd_tx.clone(),
                join_handle,
                sink: tab_handle.sink.clone(),
            },
        );

        // Increase metrics
        self.sink
            .tabs_created
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Set tab defaults
        tab_handle
            .set_title(initial.title.as_deref().unwrap_or("New Tab"))
            .await?;
        tab_handle.set_viewport(initial.viewport.unwrap_or_default()).await?;

        // Load URL in tab if provided
        if let Some(url) = initial.url.as_ref() {
            tab_handle.navigate(url).await?;
        }

        Ok(tab_handle)
    }

    /// Forwards storage events from the storage service to the engine event channel.
    fn spawn_storage_events_to_engine(&self) -> Result<tokio::task::JoinHandle<()>, EngineError> {
        let mut rx = self.context.storage_rx.resubscribe();
        let tx = self.context.event_tx.clone();
        let zone_id = self.id;

        let join_handle = spawn_named("storage-events-forwarder", async move {
            while let Ok(ev) = rx.recv().await {
                let _ = tx.send(EngineEvent::StorageChanged {
                    tab_id: ev.source_tab,
                    zone: Some(zone_id),
                    key: ev.key.unwrap_or_default(),
                    value: ev.new_value,
                    scope: ev.scope,
                    origin: ev.origin.clone(),
                });
            }
        });

        Ok(join_handle)
    }

    /// Closes a tab: asks the worker to stop and waits for it to exit.
    ///
    /// The worker performs its own teardown on exit (emits `TabClosed`, drops the
    /// tab's session storage). Returns `false` when the tab is unknown.
    pub async fn close_tab(&mut self, tab_id: TabId) -> bool {
        let Some(info) = self.tabs.remove(&tab_id) else {
            return false;
        };

        // A send error means the worker already exited; awaiting the join handle is
        // still correct in that case.
        let _ = info.cmd_tx.send(TabCommand::CloseTab).await;

        let secs = self.context.config_store.get_uint("engine.io_shutdown_secs") as u64;
        match tokio::time::timeout(std::time::Duration::from_secs(secs), info.join_handle).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => log::warn!("Tab {tab_id} worker join error: {e}"),
            // The worker keeps running detached and cleans up whenever it exits.
            Err(_) => log::warn!("Tab {tab_id} worker did not stop within {secs}s"),
        }

        true
    }

    /// Closes every tab in this zone. Used by `GosubEngine::close_zone`.
    pub(crate) async fn close(mut self) {
        let tab_ids = self.list_tabs();
        for tab_id in tab_ids {
            self.close_tab(tab_id).await;
        }
    }

    /// Lists all tab IDs in this zone.
    pub fn list_tabs(&self) -> Vec<TabId> {
        self.tabs.keys().cloned().collect()
    }
}

impl<C: RenderConfiguration> Drop for Zone<C> {
    fn drop(&mut self) {
        // Not an error: cookies persist eagerly and workers clean up after themselves,
        // but the engine keeps counting this zone against `max_zones` until
        // `GosubEngine::close_zone` is used.
        if !self.tabs.is_empty() {
            log::debug!(
                "Zone {} dropped without close_zone(); {} tab worker(s) left running detached",
                self.id,
                self.tabs.len()
            );
        }
    }
}

/// Resolves the effective tab limit for a zone: the engine-wide `engine.zone.max_tabs` setting,
/// further capped by any tighter per-zone [`ZoneConfig`] value.
fn effective_max_tabs(config: &Config, zone_max: usize) -> usize {
    config.get_uint("engine.zone.max_tabs").min(zone_max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zone::ZoneId;
    use std::collections::HashSet;
    use uuid::Uuid;

    #[test]
    fn zone_id_new_is_unique_enough() {
        let mut set = HashSet::new();
        for _ in 0..256 {
            set.insert(ZoneId::new());
        }
        assert_eq!(set.len(), 256, "ZoneId::new() should produce unique IDs");
    }

    #[test]
    fn zone_id_from_uuid_and_display_round_trip() {
        let u = Uuid::parse_str("123e4567-e89b-12d3-a456-426614174000").unwrap();
        let zid = ZoneId::from(u);
        let s = zid.to_string();
        assert_eq!(s, u.to_string(), "Display for ZoneId should mirror inner Uuid");
        // sanity: Debug contains the UUID somewhere
        let dbg = format!("{zid:?}");
        assert!(dbg.contains(&u.to_string()), "Debug for ZoneId should include UUID");
    }

    #[test]
    fn shared_flags_default_is_all_false() {
        let f = SharedFlags::default();
        assert!(!f.share_autocomplete);
        assert!(!f.share_bookmarks);
        assert!(!f.share_passwords);
        assert!(!f.share_cookiejar);
    }

    #[test]
    fn effective_max_tabs_reads_config_and_caps_with_zone() {
        use gosub_config::settings::Setting;

        let config = crate::engine::settings_store::default_config();
        let zone_default = ZoneConfig::default().max_tabs; // 16

        // Default config value (16) with the default zone value (16).
        assert_eq!(effective_max_tabs(&config, zone_default), 16);

        // Lowering the engine-wide setting takes effect.
        config.set("engine.zone.max_tabs", Setting::UInt(4)).unwrap();
        assert_eq!(effective_max_tabs(&config, zone_default), 4);

        // A tighter per-zone value still wins (we use the smaller of the two).
        assert_eq!(effective_max_tabs(&config, 2), 2);
    }
}
