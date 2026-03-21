use crate::cookies::CookieStoreHandle;
use crate::engine::cookies::CookieJarHandle;
use crate::engine::engine::EngineContext;
use crate::engine::events::EngineEvent;
use crate::engine::storage::{StorageService, Subscription};
use crate::engine::tab::TabId;
use crate::engine::types::{EventChannel, IoChannel};
use crate::storage::types::PartitionPolicy;
use crate::tab::services::resolve_tab_services;
use crate::tab::{create_tab_and_spawn, TabDefaults, TabHandle, TabOverrides, TabSink};
use gosub_interface::config::HasDocument;
use crate::util::spawn_named;
use crate::zone::ZoneConfig;
use crate::EngineError;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, RwLock};
use crate::net::req_ref_tracker::RequestReferenceMap;
use crate::render::backend::{CompositorSink, RenderBackend};

pub use gosub_net::types::ZoneId;

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
pub struct ZoneContext {
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

    /// Compositor router to use for this zone
    pub(crate) compositor: Arc<RwLock<dyn CompositorSink + Send + Sync>>,
    /// Rendering backend to use for this zone
    pub(crate) render_backend: Arc<dyn RenderBackend + Send + Sync>,
}

// Things that are shared upwards to the engine
pub struct ZoneSink {
    /// How many tabs has this zone created over its lifetime
    tabs_created: AtomicUsize,
}

/// This is the zone structure, which contains tabs and shared services. It is only known to the engine
/// and can be controlled by the user via the engine API.
pub struct Zone {
    // Shared context from the engine
    pub engine_context: Arc<EngineContext>,
    // Shared context that is passed down to tabs
    pub context: Arc<ZoneContext>,
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

impl Debug for Zone {
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
    #[allow(unused)]
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

impl Zone {
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
    ) -> Self {
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
            let guard = engine_context.io_tx.read().unwrap();
            guard.as_ref().cloned().expect("I/O thread not running")
        };
        let request_reference_map = engine_context.request_reference_map.clone();
        let compositor = engine_context.compositor.clone();
        let render_backend = engine_context.render_backend.clone();

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
        zone
    }

    /// Creates a new zone with a random ID and the provided configuration
    pub fn new(config: ZoneConfig, services: ZoneServices, engine_context: Arc<EngineContext>) -> Self {
        Self::new_with_id(ZoneId::new(), config, services, engine_context)
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
    pub async fn create_tab<C: HasDocument + Send + Sync + 'static>(
        &mut self,
        initial: TabDefaults,
        overrides: Option<TabOverrides>,
    ) -> Result<TabHandle, EngineError>
    where
        C::Document: Send + Sync,
    {
        if self.tabs.len() >= self.config.max_tabs {
            return Err(EngineError::TabLimitExceeded);
        }

        let tab_services = resolve_tab_services(
            self.id,
            &self.context.services,
            &overrides.unwrap_or_default(),
        );

        let (tab_handle, join_handle) = create_tab_and_spawn::<C>(self.id, tab_services, self.context.clone())
            .map_err(|e| EngineError::CreateTab(e.into()))?;
        self.tabs.insert(
            tab_handle.tab_id,
            TabInfo {
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
        tab_handle
            .set_viewport(initial.viewport.unwrap_or_default())
            .await?;

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

    /// Closes a tab.
    pub fn close_tab(&mut self, tab_id: TabId) -> bool {
        if let Some(_) = self.tabs.remove(&tab_id) {
            // Drop the command channel to signal the tab to close
            // drop(shared_state.cmd_tx);

            // Disconnect the session storage for this tab
            self.context.services.storage.drop_tab(self.id, tab_id);
            return true;
        }

        false
    }

    /// Lists all tab IDs in this zone.
    pub fn list_tabs(&self) -> Vec<TabId> {
        self.tabs.keys().cloned().collect()
    }
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
        assert_eq!(
            s,
            u.to_string(),
            "Display for ZoneId should mirror inner Uuid"
        );
        // sanity: Debug contains the UUID somewhere
        let dbg = format!("{zid:?}");
        assert!(
            dbg.contains(&u.to_string()),
            "Debug for ZoneId should include UUID"
        );
    }

    #[test]
    fn shared_flags_default_is_all_false() {
        let f = SharedFlags::default();
        assert!(!f.share_autocomplete);
        assert!(!f.share_bookmarks);
        assert!(!f.share_passwords);
        assert!(!f.share_cookiejar);
    }
}
