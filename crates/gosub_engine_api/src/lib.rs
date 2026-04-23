// #![deny(missing_docs)]
// #![deny(rustdoc::broken_intra_doc_links)]

//! # Gosub Engine
//!
//! Gosub is a work-in-progress, embeddable browser engine for building your own User Agent (UA).
//! It uses **async channels** and **handles**:
//! - `EngineEvent` flows from the engine → UA over an event channel.
//! - You control things via `EngineCommand` (engine/zone scoped) and `TabCommand` (tab scoped).
//! - The engine owns a **render backend** (e.g., Null, Cairo, Vello) that you provide.
//! - The engine is built around a **multi-zone** model, where each zone represents a separate profile.
//! - A compositor(sink) is owned by the UA and receives `Redraw` events to composite into the final UI.
//! - Each zone can have multiple tabs (browsing contexts).
//! - Zones own their own cookies and storage.
//! - Tabs are controlled via a `TabHandle`.
//! - Tabs emit events (navigation, resource loading, rendering) that you can handle in your UA.
//! - The engine is designed to be **modular** and **extensible**.
//! - You can plug in your own networking stack, render backend, storage backend, etc.
//! - The engine is built using **Tokio** and **async/await**.
//! - It is designed to be **thread-safe** and **concurrent**.
//! - The engine is still a work in progress and is not yet production-ready.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use url::Url;
//!
//! use gosub_engine_api::{EngineConfig, GosubEngine};
//! use gosub_engine_api::render::Viewport;
//! use gosub_engine_api::render::backends::null::NullBackend;
//! use gosub_engine_api::events::{EngineEvent, TabCommand};
//! use gosub_engine_api::storage::{StorageService, InMemoryLocalStore, InMemorySessionStore, PartitionPolicy};
//! use gosub_engine_api::cookies::DefaultCookieJar;
//! use gosub_engine_api::zone::{ZoneConfig, ZoneServices};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // 1) Engine + backend
//!     let backend = NullBackend::new().expect("null renderer cannot be created (!?)");
//!     let compositor = gosub_engine_api::render::DefaultCompositor::default();
//!     let mut engine_handle = GosubEngine::new(
//!         Some(EngineConfig::default()),
//!         Arc::new(backend),
//!         Arc::new(std::sync::RwLock::new(compositor)),
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
//!     };
//!
//!     // 3) Create a zone (ZoneHandle)
//!     let mut zone = engine_handle.create_zone(ZoneConfig::default(), services, None)?;
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
//!                if let gosub_engine_api::events::NavigationEvent::Started { url, .. } = event {
//!                    println!("[{tab_id:?}] Starting loading: {url}");
//!                }
//!             }
//!             EngineEvent::Redraw { tab_id, .. } => {
//!                 // Composite `handle` into your UI
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
//! ## Concepts
//! - [`GosubEngine`] — engine entry point; creates zones, owns backend and event bus.
//! - [`Zone`](crate::zone::Zone) — per-profile/session state (cookies, storage, tabs). Owned by the caller.
//! - [`TabHandle`](crate::tab) — a single browsing context controlled via [`TabCommand`](crate::events::TabCommand).
//! - [`RenderBackend`](crate::render::backend::RenderBackend) — pluggable renderer (e.g., Null, Cairo, Vello).
//!
//! ## Persistence
//! To persist cookies, pass a [`CookieStore`](crate::cookies::CookieStore) in
//! `ZoneServices::cookie_store` and omit `cookie_jar`; the engine will attach a per-zone
//! [`PersistentCookieJar`](crate::cookies::PersistentCookieJar).

extern crate core;

mod engine;

pub mod net;

pub mod render;

pub mod util;

pub mod html;

pub use engine::{EngineError, GosubEngine};

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


/// Public `events` namespace with the enums/structs:
pub mod events {
    pub use crate::engine::events::{EngineCommand, EngineEvent, IoCommand, MouseButton, TabCommand};
    pub use crate::engine::events::{NavigationEvent, ResourceEvent};
}

/// Configuration options for the Gosub engine.
pub mod config {
    pub use crate::engine::config::{
        CookiePartitioning, GpuOptions, LogLevel, ProxyConfig, RedirectPolicy, SandboxMode, TlsConfig,
    };
}