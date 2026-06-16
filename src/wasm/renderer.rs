use crate::wasm::wgpu_context::WasmContextProvider;
use gosub_engine::cookies::DefaultCookieJar;
use gosub_engine::events::TabCommand;
use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
use gosub_engine::tab::TabDefaults;
use gosub_engine::zone::{ZoneConfig, ZoneServices};
use gosub_engine::DefaultRenderConfig;
use gosub_engine::GosubEngine;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::{DefaultCompositor, Viewport};
use gosub_renderer_vello::VelloBackend;
use gosub_shared::tab_id::TabId;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::LocalSet;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

/// A running Gosub browser instance attached to a canvas element.
#[wasm_bindgen]
pub struct GosubBrowser {
    // Kept alive for the lifetime of the JS object.
    #[allow(dead_code)]
    engine: GosubEngine,
}

#[wasm_bindgen]
impl GosubBrowser {
    /// Attach a browser to the canvas element with id `canvas_id` and navigate to `url`.
    ///
    /// `width` / `height` are the initial viewport in CSS pixels.
    ///
    /// Internally, all engine tasks run on a Tokio `LocalSet` driven by the JS event loop.
    /// `spawn_named` uses `tokio::task::spawn_local` on WASM, which requires this LocalSet
    /// to be active. The LocalSet is kept alive by a `spawn_local` future that runs indefinitely.
    #[wasm_bindgen(constructor)]
    pub async fn new(canvas_id: String, url: String, width: u32, height: u32) -> Result<GosubBrowser, JsValue> {
        let provider = Arc::new(
            WasmContextProvider::new(&canvas_id, width, height)
                .await
                .map_err(|e| JsValue::from_str(&e))?,
        );

        let backend: Arc<dyn gosub_render_pipeline::render::RenderBackend + Send + Sync> =
            Arc::new(VelloBackend::new(Arc::clone(&provider)).map_err(|e| JsValue::from_str(&e.to_string()))?);

        // Late-fill slot: the compositor callback fires after submit_frame stores a frame,
        // so frames_arc() is valid by the time the callback is first invoked.
        let frames_slot: Arc<RwLock<Option<Arc<RwLock<HashMap<TabId, ExternalHandle>>>>>> = Arc::new(RwLock::new(None));
        let frames_slot_cb = Arc::clone(&frames_slot);
        let provider_cb = Arc::clone(&provider);

        let compositor = Arc::new(RwLock::new(DefaultCompositor::new(move || {
            let slot = frames_slot_cb.read();
            if let Some(ref frames) = *slot {
                let map = frames.read();
                for handle in map.values() {
                    if let ExternalHandle::WgpuTextureId { id, .. } = handle {
                        provider_cb.present(*id);
                        break;
                    }
                }
            }
        })));
        *frames_slot.write() = Some(compositor.read().frames_arc());

        // Channel to move the engine out of the LocalSet context after setup.
        let (engine_tx, engine_rx) = tokio::sync::oneshot::channel::<Result<GosubEngine, String>>();

        // All engine tasks use spawn_named → spawn_local on WASM, which requires an active
        // LocalSet. We run setup inside run_until, then keep the set alive indefinitely.
        let local = LocalSet::new();

        let backend_clone = Arc::clone(&backend);
        let compositor_clone = Arc::clone(&compositor);

        local.spawn_local(async move {
            let result = async move {
                let mut engine = GosubEngine::<DefaultRenderConfig<_>>::new(None, backend_clone, compositor_clone);
                engine.start().map_err(|e| e.to_string())?;

                let services = ZoneServices {
                    storage: Arc::new(StorageService::new(
                        Arc::new(InMemoryLocalStore::new()),
                        Arc::new(InMemorySessionStore::new()),
                    )),
                    cookie_store: None,
                    cookie_jar: Some(DefaultCookieJar::new().into()),
                    partition_policy: PartitionPolicy::None,
                };

                let mut zone = engine
                    .create_zone(ZoneConfig::default(), services, None)
                    .map_err(|e| e.to_string())?;

                let tab = zone
                    .create_tab(
                        TabDefaults {
                            url: None,
                            title: Some("New Tab".to_string()),
                            viewport: Some(Viewport::new(0, 0, width, height)),
                        },
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                tab.send(TabCommand::ResumeDrawing { fps: 60 })
                    .await
                    .map_err(|e| e.to_string())?;

                tab.navigate(&url).await.map_err(|e| e.to_string())?;

                Ok::<GosubEngine, String>(engine)
            }
            .await;

            let _ = engine_tx.send(result);
        });

        // Drive the LocalSet until setup completes.
        local.run_until(async {}).await;

        // Keep all spawned engine tasks running on the JS event loop.
        spawn_local(async move {
            local.run_until(std::future::pending::<()>()).await;
        });

        let engine = engine_rx
            .await
            .map_err(|_| JsValue::from_str("engine setup task dropped"))?
            .map_err(|e| JsValue::from_str(&e))?;

        Ok(GosubBrowser { engine })
    }
}
