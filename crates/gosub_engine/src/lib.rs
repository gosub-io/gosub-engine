// #![deny(missing_docs)]
// #![deny(rustdoc::broken_intra_doc_links)]

//! # Gosub Engine
//!
//! Gosub is a work-in-progress, embeddable browser engine for building your own User Agent (UA).
//! It uses **async channels** and **handles**:
//! - `EngineEvent` flows from the engine → UA over an event channel.
//! - You control tabs via `TabCommand`; the engine itself via methods on [`GosubEngine`].
//! - The engine owns a **render backend** (e.g., Null, Cairo, Vello) that you provide.
//! - The engine is built around a **multi-zone** model, where each zone represents a separate profile.
//! - A compositor(sink) is owned by the UA and receives the finished frames to composite into the final UI.
//! - Each zone can have multiple tabs (browsing contexts).
//! - Zones own their own cookies and storage.
//! - Tabs are controlled via a `TabHandle`.
//! - Tabs emit events (navigation, resource loading, rendering) that you can handle in your UA.
//! - The engine is built on **Tokio**; render backend, storage backend, and cookie store are pluggable.
//! - The engine is still a work in progress and is not yet production-ready.
//!
//! ## The `unstable-api` feature
//!
//! Everything reachable in a default build is wired up: every [`EngineEvent`](crate::events::EngineEvent)
//! variant is emitted somewhere, and every [`TabCommand`](crate::events::TabCommand) variant
//! reaches a handler. Variants that are declared but never emitted, or accepted and dropped,
//! sit behind the non-default `unstable-api` feature. Matching one without the feature is a
//! compile error rather than a runtime no-op.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use url::Url;
//!
//! use gosub_engine::{EngineConfig, GosubEngine};
//! use gosub_render_pipeline::render::Viewport;
//! use gosub_render_pipeline::render::backends::null::NullBackend;
//! use gosub_render_pipeline::render::DefaultCompositor;
//! use gosub_engine::events::{EngineEvent, TabCommand};
//! use gosub_engine::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine::cookies::DefaultCookieJar;
//! use gosub_engine::zone::{ZoneConfig, ZoneServices};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // 1) Engine + backend
//!     let backend = NullBackend::new();
//!     let compositor = DefaultCompositor::default();
//!     let mut engine_handle: GosubEngine = GosubEngine::new(
//!         Some(EngineConfig::default()),
//!         Arc::new(backend),
//!         Arc::new(compositor),
//!     );
//!
//!     // 2) Zone services (ephemeral cookies here; use a CookieStore for persistence)
//!     let services = ZoneServices {
//!         storage: Arc::new(StorageService::new(
//!             Arc::new(InMemoryLocalStore::new()),
//!             Arc::new(InMemorySessionStore::new()),
//!         )),
//!         cookie_store: None,
//!         cookie_jar: Some(DefaultCookieJar::new().into()),
//!         partition_policy: PartitionPolicy::None,
//!         places: None,
//!     };
//!
//!     // 3) Create a zone (ZoneHandle)
//!     let mut zone = engine_handle.create_zone(None, services, None)?;
//!
//!     // 4) Create a tab (TabHandle)
//!     let tab_handle = zone.create_tab(Default::default(), None).await?;
//!
//!     // 5) Drive the tab
//!     tab_handle.send(TabCommand::Navigate{ url: "https://example.com".to_string() }).await?;
//!     tab_handle.send(TabCommand::SetViewport{ x: 0, y: 0, width: 1280, height: 800 }).await?;
//!
//!     // 6) Handle engine events in your UA
//!     let mut event_rx = engine_handle.subscribe_events();
//!     while let Ok(ev) = event_rx.recv().await {
//!         match ev {
//!             EngineEvent::Navigation { tab_id, event } => {
//!                if let gosub_engine::events::NavigationEvent::Started { url, .. } = event {
//!                    println!("[{tab_id:?}] Starting loading: {url}");
//!                }
//!             }
//!             EngineEvent::Redraw { tab_id } => {
//!                 // Doorbell only: ask your compositor sink for this tab's frame and
//!                 // present it. See "Frames and scrolling" below.
//!                 println!("[{tab_id:?}] Redraw requested");
//!             }
//!             _ => {}
//!         }
//!     }
//!
//!     engine_handle.shutdown().await;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Frames and scrolling
//!
//! ### Frames
//!
//! Frames leave the engine on the compositor sink; the event bus only says that one is
//! ready. When a tab finishes a frame the worker calls
//! [`CompositorSink::submit_frame`](gosub_render_pipeline::render::backend::CompositorSink::submit_frame)
//! with the backend's `ExternalHandle`, then emits [`EngineEvent::Redraw`](crate::events::EngineEvent::Redraw) carrying just the
//! `TabId`.
//!
//! The sink is the data channel: it holds the pixels (or the GPU texture id, or the tile
//! cache), you passed it to [`GosubEngine::new`], and you read the current frame out of it
//! when you paint. `Redraw` is only the wakeup - it carries no frame data, and maps onto the
//! toolkit's invalidate call (`queue_draw`, `request_repaint`, `window.request_redraw`).
//!
//! Present from the sink, not from the event. The bus is a [`tokio::sync::broadcast`]
//! channel, so a slow consumer drops messages under load. A dropped wakeup costs one
//! coalesced repaint, because the next one still finds the newest frame in the sink; a
//! dropped frame would cost the frame.
//!
//! ### Scrolling
//!
//! Both sides track the offset. The engine is authoritative: it clamps to the page, restores
//! per-history-entry offsets, and runs the smooth-scroll animation. A shell may also keep its
//! own offset so it can move already-rasterized tiles at input rate without a round trip
//! through the tab worker.
//!
//! The two are reconciled through the frame: the tile cache handed to the sink carries the
//! offset it was composited at, so a shell drawing at its own local offset converges on the
//! engine's as frames arrive.
//!
//! Which command you send says who is deciding:
//!
//! - [`TabCommand::MouseScroll`](crate::events::TabCommand::MouseScroll) hands the engine a
//!   delta and lets it decide where that lands, including animating toward it. Use this for
//!   raw wheel and trackpad input.
//! - [`TabCommand::SetScroll`](crate::events::TabCommand::SetScroll) tells the engine an
//!   absolute offset and cancels any animation in flight. Use this when the shell is the one
//!   that knows: a scrollbar drag, a shell-side kinetic/smooth scroll, or a restored session.
//!
//! Do not mix the two within one gesture; an absolute set landing mid-animation fights the
//! animator.
//!
//! ## Concepts
//! - [`GosubEngine`] - engine entry point; creates zones, owns backend and event bus.
//! - [`Zone`](crate::zone::Zone) - per-profile/session state (cookies, storage, tabs). Owned by the caller.
//! - [`TabHandle`](crate::tab) - a single browsing context controlled via [`TabCommand`](crate::events::TabCommand).
//!   Commands go in asynchronously; the tab's current URL, title and back/forward
//!   availability read back synchronously off the handle
//!   ([`url`](crate::tab::TabHandle::url), [`title`](crate::tab::TabHandle::title),
//!   [`can_go_back`](crate::tab::TabHandle::can_go_back)), so a shell can build a tab strip
//!   or restore a session without replaying the event stream.
//! - [`RenderBackend`](gosub_render_pipeline::render::backend::RenderBackend) - pluggable renderer (e.g., Null, Cairo, Vello).
//!
//! ## Configuration - choosing your components
//!
//! The engine is generic over a single *configuration* type that names every pluggable
//! component at compile time (there is no runtime registry - naming `CairoBackend` is what
//! pulls Cairo into your build). It comes in two layers:
//!
//! - [`ModuleConfiguration`](gosub_interface::config::ModuleConfiguration) - the parse/style
//!   stack: CSS system, DOM document, HTML parser. Parse-only tools (test harnesses, fuzzers)
//!   that never paint implement only this.
//! - [`RenderConfiguration`](crate::html::RenderConfiguration) - extends `ModuleConfiguration`
//!   with the runtime render components: the [`RenderBackend`](gosub_render_pipeline::render::backend::RenderBackend),
//!   the compositor sink, and the font system. Anything that actually renders needs this.
//!
//! You almost never implement these by hand. [`DefaultRenderConfig<B, F, S>`](crate::DefaultRenderConfig)
//! is a ready-made zero-sized marker that wires the standard gosub stack (`gosub_html5` +
//! `gosub_css3`) and lets you pick the parts that vary:
//! - `B` - render backend (`CairoBackend`, `SkiaBackend`, `VelloBackend`, `NullBackend`, …)
//! - `F` - font system (defaults to `ParleyFontSystem`)
//! - `S` - compositor sink (defaults to [`DefaultCompositor`](gosub_render_pipeline::render::DefaultCompositor))
//!
//! **To start a browser that renders**, alias your chosen stack once and hand it to the engine:
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use gosub_engine::{GosubEngine, DefaultRenderConfig};
//! use gosub_renderer_cairo::{CairoBackend, PangoFontSystem};
//!
//! // Pick a backend + font system once; reuse the alias everywhere (struct fields, fn sigs).
//! type AppConfig = DefaultRenderConfig<CairoBackend, PangoFontSystem>;
//!
//! let mut engine = GosubEngine::<AppConfig>::new(None, Arc::new(backend), compositor);
//! ```
//!
//! With no type parameters, `DefaultRenderConfig` is the headless
//! `DefaultRenderConfig<NullBackend, ParleyFontSystem, DefaultCompositor>` used in the Quick start
//! above - which is why that example needs no backend choice. For complete winit / GTK4 / egui
//! setups with each backend, see the `examples/` directory.
//!
//! ## Persistence
//! To persist cookies, pass a [`CookieStore`](crate::cookies::CookieStore) in
//! `ZoneServices::cookie_store` and omit `cookie_jar`; the engine will attach a per-zone
//! [`PersistentCookieJar`](crate::cookies::PersistentCookieJar).

extern crate core;

mod engine;

pub mod net;

pub mod util;

pub mod html;

#[cfg(feature = "metrics")]
pub mod metrics;

pub use engine::{BrowsingContext, EngineError, GosubEngine};

/// The engine's ready-made config: a marker that implements both
/// [`ModuleConfiguration`](gosub_interface::config::ModuleConfiguration) (parse/style stack) and
/// [`RenderConfiguration`](html::RenderConfiguration) (render components), parameterized over the
/// render backend, font system, and compositor sink. Used when `GosubEngine` is instantiated
/// without a custom config. See the crate-level "Configuration" section for how to pick a backend.
pub use html::DefaultRenderConfig;

/// Builds a [`gosub_config::Config`] seeded with the engine's built-in settings schema.
pub use engine::default_settings;

/// `gosub://` internal pages: the registry embedders extend/override (see [`GosubEngine::internal_pages`]).
pub use engine::internal_pages;

/// Bookmarks + visited history ("places"), per zone: the store type shells share.
pub use engine::places;

/// The engine's settings store and its value/schema types (see [`GosubEngine::settings`]).
pub use gosub_config::settings::{Constraint, Setting, SettingInfo};
/// Storage adapters an embedder can attach to the settings store to persist overrides.
pub use gosub_config::storage as config_storage;
pub use gosub_config::Config;
pub use gosub_config::StorageAdapter;

pub use engine::types::Action;
pub use engine::types::NavigationId;

#[doc(inline)]
/// Tab management and browsing context API.
pub use engine::tab;

/// Per-profile/session state (cookies, storage, tabs).
#[doc(inline)]
pub use engine::zone;

#[doc(inline)]
pub use engine::cookies;

#[doc(inline)]
/// Storage APIs for local/session data.
pub use engine::storage;

// EngineConfig at crate root:
#[doc(inline)]
pub use crate::engine::config::EngineConfig;

#[doc(inline)]
pub use crate::engine::cookies::SameSiteContext;
pub use crate::engine::cookies::ThirdPartyCookiePolicy;

/// Public `events` namespace with the enums/structs:
pub mod events {
    pub use crate::engine::events::{
        CursorShape, DownloadId, EngineEvent, HitTestResponse, HitTestToken, Modifiers, MouseButton, TabCommand,
    };
    pub use crate::engine::events::{NavigationEvent, ResourceEvent};
}

/// Configuration options for the Gosub engine.
pub mod config {
    pub use crate::engine::config::{EngineConfig, EngineConfigBuilder, EngineConfigError};
}
