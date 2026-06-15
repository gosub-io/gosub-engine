use crate::cookies::SameSiteContext;
use crate::engine::errors::NavigationError;
use crate::engine::events::{EngineEvent, NavigationEvent, TabInternalCommand};
use crate::engine::resource_pipeline::ResourcePipelines;
use crate::engine::types::{NavigationId, RequestId};
use crate::engine::{BrowsingContext, UaPolicy};
use crate::events::{IoCommand, TabCommand};
use crate::html::EngineConfig;
use crate::net::req_ref_tracker::RequestReference;
use crate::net::types::{FetchKeyData, FetchRequest, FetchResult, Initiator, NetError, Priority, ResourceKind};
use crate::net::{route_response_for, submit_to_io, RequestDestination, RoutedOutcome};
use crate::storage::types::compute_partition_key;
use crate::storage::{StorageEvent, StorageHandles};
use crate::tab::services::EffectiveTabServices;
use crate::tab::state::{TabActivityMode, TabRuntime, TabState};
use crate::tab::{TabId, TabSink};
use crate::util::spawn_named;
use crate::zone::{ZoneContext, ZoneId};
use anyhow::{anyhow, Context};
use gosub_render_pipeline::rasterizer::RasterStrategy;
use gosub_render_pipeline::render::backend::{
    CompositorSink, ErasedSurface, PresentMode, RenderBackend, RgbaImage, SurfaceSize,
};
use gosub_render_pipeline::render::Viewport;
use http::{HeaderMap, Method};
use std::sync::Arc;
use tokio::select;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Fallback URL used when a navigation has no usable URL.
fn about_blank() -> Url {
    #[allow(clippy::unwrap_used)] // PANIC-SAFE: literal URL
    Url::parse("about:blank").unwrap()
}

#[derive(Debug)]
pub enum NavigationResult<C: EngineConfig> {
    Ok {
        nav_id: NavigationId,
        final_url: Url,
        title: Option<String>,
        doc: Arc<crate::html::EngineDocument<C>>,
    },
    Err {
        nav_id: NavigationId,
        error: NavigationError,
    },
}

// Current active navigation
struct ActiveNav {
    pub nav_id: NavigationId,
    pub cancel: CancellationToken,
    pub url: Url,
}

struct NavJoin<C: EngineConfig> {
    // nav_id: NavigationId,
    cancel: CancellationToken,
    // Wrapped in Option so the receiver can be extracted into `pending_nav_rx`
    // without dropping the cancel token from self.load.
    rx: Option<oneshot::Receiver<NavigationResult<C>>>,
}

pub struct TabWorker<C: EngineConfig> {
    /// ID of the tab
    pub tab_id: TabId,
    /// ID of the zone in which this tab resides
    pub zone_id: ZoneId,

    /// Shared context from the tab
    zone_context: Arc<ZoneContext<C>>,
    // Effective tab services that we can use
    services: EffectiveTabServices,

    /// Sink for sending events upwards
    sink: Arc<TabSink>,

    /// Receiver for incoming tab commands
    cmd_rx: mpsc::Receiver<TabCommand>,

    /// Browsing context running for this tab
    pub context: BrowsingContext<C>,
    /// State of the tab (idle, loading, loaded, etc.)
    pub state: TabState,
    /// Current tab mode (idle, live, background)
    pub mode: TabActivityMode,

    /// Favicon binary data for the current tab
    pub favicon: Vec<u8>,
    /// Title of the current tab
    pub title: String,
    /// URL that ready to load or is loading
    pub pending_url: Option<Url>,
    /// Current URL that is now loaded
    pub current_url: Option<Url>,
    /// Is the current URL being loaded
    pub is_loading: bool,
    /// Is there an error in the current tab?
    pub is_error: bool,

    // ** Backend rendering

    // Thumbnail image of the tab in case the tab is not visible
    pub thumbnail: Option<RgbaImage>,
    // Surface on which the browsing context can render the tab
    surface: Option<Box<dyn ErasedSurface + Send>>,
    // // Size of the surface (does not have to match viewport)
    // surface_size: SurfaceSize,
    // Present mode for the surface?
    present_mode: PresentMode,
    /// The newest viewport requested by the tab, which may differ from the committed one.
    desired_viewport: Viewport,
    /// Current scroll offset in CSS pixels (updated by MouseScroll).
    scroll_x: i32,
    scroll_y: i32,
    /// Keeps track of the tab worker runtime data
    pub(crate) runtime: TabRuntime,
    /// Current in-flight navigation (if any)
    load: Option<NavJoin<C>>,
    /// Current active navigation (if any)
    active_nav: Option<ActiveNav>,

    // Send channel for internal commands (reserved for future worker-internal use)
    #[allow(dead_code)]
    internal_tx: mpsc::Sender<TabInternalCommand>,
    // Receive channel for internal commands
    internal_rx: mpsc::Receiver<TabInternalCommand>,
}

impl<C: EngineConfig> TabWorker<C> {
    /// Creates a new tab. Does NOT spawn the tab worker
    pub fn new(
        tab_id: TabId,
        zone_id: ZoneId,
        services: EffectiveTabServices,
        zone_context: Arc<ZoneContext<C>>,
        sink: Arc<TabSink>,
        cmd_rx: mpsc::Receiver<TabCommand>,
    ) -> Self {
        let (internal_tx, internal_rx) = mpsc::channel::<TabInternalCommand>(32);

        Self {
            tab_id,
            zone_id,
            services,
            zone_context,
            sink,
            cmd_rx,
            context: BrowsingContext::new(),
            state: TabState::Idle,
            mode: TabActivityMode::Active,
            favicon: vec![],
            title: "New Tab".to_string(),
            pending_url: None,
            current_url: None,
            is_loading: false,
            is_error: false,
            thumbnail: None,
            surface: None,
            present_mode: PresentMode::Fifo,
            desired_viewport: Default::default(),
            scroll_x: 0,
            scroll_y: 0,
            runtime: TabRuntime::default(),
            load: None,
            active_nav: None,
            internal_tx,
            internal_rx,
        }
    }

    /// Spawns the tab worker into a new task and returns the join handle
    pub fn spawn_worker(self) -> anyhow::Result<JoinHandle<()>> {
        let name = format!("Tab Worker {}", self.tab_id);
        let join_handle = spawn_named(&name, self.run_worker());

        Ok(join_handle)
    }

    // Main loop of the tab worker
    async fn run_worker(mut self) {
        self.sink.set_worker_started_now();

        // Announce creation
        self.send_event(EngineEvent::TabCreated {
            tab_id: self.tab_id,
            zone_id: self.zone_id,
        });

        // Store the nav-result receiver OUTSIDE the select! loop so it survives across
        // iterations even when another arm fires first.  oneshot::Receiver is Unpin, so
        // `&mut pending_nav_rx.as_mut().unwrap()` is a stable borrow we can reuse.
        let mut pending_nav_rx: Option<oneshot::Receiver<NavigationResult<C>>> = None;

        loop {
            // Sync pending_nav_rx with self.load so a freshly-set load is picked up.
            // Only take the receiver; leave self.load so the cancel token remains
            // reachable for CancelNavigation commands.
            if pending_nav_rx.is_none() {
                if let Some(load) = self.load.as_mut() {
                    if let Some(rx) = load.rx.take() {
                        pending_nav_rx = Some(rx);
                    }
                }
            }

            select! {
                // Handle tick for redraws
                _ = self.runtime.interval.tick(), if self.runtime.drawing_enabled => {
                    if let Err(e) = self.tick_draw().await {
                        self.state = TabState::Failed(format!("Tab {:?} tick error: {}", self.tab_id, e));
                        self.runtime.dirty = true;
                    }
                }

                // In-flight load completion — uses a persistent receiver so it is not
                // dropped when another arm fires in the same select! invocation.
                result = async {
                    match pending_nav_rx.as_mut() {
                        Some(rx) => rx.await,
                        None => std::future::pending().await,
                    }
                }, if pending_nav_rx.is_some() => {
                    pending_nav_rx = None;
                    match result {
                        Ok(res) => self.on_nav_result(res),
                        Err(e) => {
                            log::error!("Tab {:?} load receive error: {}", self.tab_id, e);
                        }
                    }
                }

                // Handle internal commands send by the tab itself
                msg = self.internal_rx.recv() => {
                    let Some(cmd) = msg else { break; };
                    self.handle_internal_command(cmd);
                }

                // Handle incoming tab commands from the UA
                msg = self.cmd_rx.recv() => {
                    let Some(cmd) = msg else { break; };
                    if self.handle_tab_command(cmd).is_break() {
                        break;
                    }
                    // If the command (e.g. hover change) requested an immediate render,
                    // call tick_draw now instead of waiting up to 1/fps seconds for the tick.
                    if std::mem::replace(&mut self.runtime.render_now, false) {
                        if let Err(e) = self.tick_draw().await {
                            self.state = TabState::Failed(format!("Tab {:?} immediate render error: {}", self.tab_id, e));
                            self.runtime.dirty = true;
                        }
                    }
                }
            }
        }

        // Receiver may already be gone at shutdown; that is expected.
        let _ = self.zone_context.event_tx.send(EngineEvent::TabClosed {
            tab_id: self.tab_id,
            zone_id: self.zone_id,
        });
        self.services.storage.drop_tab(self.zone_id, self.tab_id);
    }

    fn on_nav_result(&mut self, res: NavigationResult<C>) {
        match res {
            NavigationResult::Ok {
                nav_id,
                final_url,
                title,
                doc,
            } => {
                self.context.set_document(Arc::clone(&doc));
                self.current_url = Some(final_url.clone());
                if let Some(t) = title {
                    self.title = t;
                }
                self.is_loading = false;
                self.is_error = false;
                self.state = TabState::Idle;
                self.runtime.dirty = true;

                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::Finished { nav_id, url: final_url },
                });
            }
            NavigationResult::Err { nav_id, error } => {
                self.is_loading = false;
                self.is_error = true;
                self.state = TabState::Failed(error.to_string());
                self.runtime.dirty = true;

                let url = self
                    .active_nav
                    .as_ref()
                    .map(|a| a.url.clone())
                    .or_else(|| self.pending_url.clone())
                    .unwrap_or_else(about_blank);

                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::Failed {
                        nav_id: Some(nav_id),
                        url,
                        error: Arc::new(error.into()),
                    },
                });
            }
        }
    }

    fn handle_internal_command(&mut self, _cmd: TabInternalCommand) {}

    fn handle_tab_command(&mut self, cmd: TabCommand) -> ControlFlow {
        match cmd {
            TabCommand::CloseTab => ControlFlow::Break,
            TabCommand::Navigate { url } => {
                self.navigate_to(&url, false);
                ControlFlow::Continue
            }
            TabCommand::Reload { ignore_cache } => {
                let url = self
                    .current_url
                    .as_ref()
                    .map(|u| u.as_str())
                    .unwrap_or("about:blank")
                    .to_string();
                self.navigate_to(url.as_str(), ignore_cache);
                ControlFlow::Continue
            }
            TabCommand::SetViewport {
                x: _,
                y: _,
                width,
                height,
            } => {
                self.set_viewport(Viewport::new(0, 0, width, height));
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::MouseScroll { delta_x, delta_y } => {
                // When page height is known, clamp to the real maximum so worker and context
                // stay in sync. When the page hasn't rendered yet, allow free scrolling (the
                // context will clamp to the actual page height on its own).
                let max_y = {
                    let ph = self.context.page_height();
                    if ph > 0.0 {
                        (ph - self.desired_viewport.height as f64).max(0.0) as i32
                    } else {
                        i32::MAX / 2
                    }
                };
                let prev_x = self.scroll_x;
                let prev_y = self.scroll_y;
                self.scroll_x = (self.scroll_x + delta_x as i32).max(0);
                self.scroll_y = (self.scroll_y + delta_y as i32).clamp(0, max_y);
                self.context.set_scroll(self.scroll_x as f64, self.scroll_y as f64);

                // Submit the scroll frame immediately — don't wait for the next timer tick.
                // Eliminates up to 33ms of latency at 30fps per scroll event.
                if self.zone_context.render_backend.raster_strategy() != RasterStrategy::None {
                    let dpr = self.zone_context.render_backend.device_pixel_ratio();
                    if let Some(handle) = self.context.take_scroll_handle(dpr) {
                        self.runtime.committed_scene_epoch = self.context.scene_epoch();
                        self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
                        return ControlFlow::Continue;
                    }
                }

                // TileCache not ready yet (first render hasn't completed); fall back to timer path.
                // Only mark dirty if the scroll position actually moved (sub-pixel deltas are no-ops).
                if self.scroll_x != prev_x || self.scroll_y != prev_y {
                    self.runtime.dirty = true;
                }
                ControlFlow::Continue
            }
            TabCommand::MouseMove { x, y } => {
                // Process the hit-test immediately so hover doesn't wait for the next tick.
                let (visual_dirty, url_changed, link_url) = self.context.update_hover(x as f64, y as f64);
                if url_changed {
                    self.send_event(EngineEvent::HoverUrl {
                        tab_id: self.tab_id,
                        url: link_url,
                    });
                }
                if visual_dirty {
                    self.runtime.dirty = true;
                    self.runtime.render_now = true;
                }
                ControlFlow::Continue
            }
            TabCommand::MouseDown { button, .. } => {
                if matches!(button, crate::events::MouseButton::Left) {
                    if let Some(href) = self.context.hover_link_url.clone() {
                        let resolved = self
                            .current_url
                            .as_ref()
                            .and_then(|base| base.join(&href).ok())
                            .map(|u| u.to_string())
                            .unwrap_or(href);
                        self.navigate_to(resolved, false);
                        return ControlFlow::Continue;
                    }
                }
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::MouseUp { .. }
            | TabCommand::KeyDown { .. }
            | TabCommand::KeyUp { .. }
            | TabCommand::CharInput { .. } => {
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::ResumeDrawing { fps: wanted_fps } => {
                self.runtime.drawing_enabled = true;
                self.runtime.fps = wanted_fps.max(1) as u32;
                let period = Duration::from_secs_f64(1.0 / (self.runtime.fps as f64));
                self.runtime.interval = tokio::time::interval(period);
                self.runtime
                    .interval
                    .set_missed_tick_behavior(MissedTickBehavior::Delay);
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::SuspendDrawing => {
                self.runtime.drawing_enabled = false;
                ControlFlow::Continue
            }
            TabCommand::CancelNavigation => {
                if let Some(load) = self.load.take() {
                    log::warn!("**** Cancelling in-flight load for tab {:?}", self.tab_id);
                    load.cancel.cancel();
                }
                ControlFlow::Continue
            }
            TabCommand::SubmitDecision {
                decision_token, action, ..
            } => {
                // Proxy the submit decision to the I/O thread
                let _ = self.zone_context.io_tx.send(IoCommand::Decision {
                    zone_id: self.zone_id,
                    token: decision_token,
                    action,
                });

                // Decisions are handled in the fetcher/io thread, so we can ignore this here
                ControlFlow::Continue
            }
            _ => {
                log::warn!("Tab {:?} received unhandled command: {:?}", self.tab_id, cmd);
                ControlFlow::Continue
            }
        }
    }

    /// Send an engine event upwards to the UA
    fn send_event(&self, evt: EngineEvent) {
        match self.zone_context.event_tx.send(evt.clone()) {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error sending event: {}: {:?}", e, evt);
            }
        }
    }

    /// Navigate to a new URL, cancelling any in-flight navigation.
    fn navigate_to(&mut self, url: impl Into<String>, _ignore_cache: bool) {
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.context.reset_scroll();
        // Cancel any previous running navigation in this tab
        self.cancel_current_nav();

        let url = match self.parse_url(url.into()) {
            Ok(u) => u,
            Err(_) => return,
        };

        if let Err(e) = self.bind_storage_for(url.clone()) {
            self.send_event(EngineEvent::Navigation {
                tab_id: self.tab_id,
                event: NavigationEvent::Failed {
                    nav_id: None,
                    url: url.clone(),
                    error: Arc::new(e),
                },
            });
            return;
        }

        let nav_id = NavigationId::new();
        let parent_cancel = CancellationToken::new();
        self.active_nav = Some(ActiveNav {
            nav_id,
            cancel: parent_cancel.clone(),
            url: url.clone(),
        });

        {
            let mut guard = self.zone_context.request_reference_map.write();
            guard.insert(RequestReference::Navigation(nav_id), self.tab_id);
        }

        self.sink.set_nav(nav_id);
        self.pending_url = Some(url.clone());
        self.is_loading = true;
        self.is_error = false;
        self.state = TabState::Loading;
        self.runtime.dirty = true;

        self.send_event(EngineEvent::Navigation {
            tab_id: self.tab_id,
            event: NavigationEvent::Started {
                nav_id,
                url: url.clone(),
            },
        });

        // Attach cookies for the navigation request.
        let mut fetch_headers = HeaderMap::new();
        if let Some(cookie_str) =
            self.services
                .cookie_jar
                .read()
                .get_request_cookies(&url, Some(&url), SameSiteContext::SameSite)
        {
            if let Ok(val) = cookie_str.parse() {
                fetch_headers.insert(http::header::COOKIE, val);
            }
        }

        let req = FetchRequest {
            reference: RequestReference::Navigation(nav_id),
            req_id: RequestId::new(),
            key_data: FetchKeyData {
                url: url.clone(),
                method: Method::GET,
                headers: fetch_headers,
            },
            priority: Priority::High,
            kind: ResourceKind::Document,
            initiator: Initiator::Navigation,
            // Use buffered mode so the full document body is available before parsing.
            // The streaming path has a race where SharedBody can close before parse_stream
            // subscribes, causing truncated HTML (only the 5 KB peek buffer is parsed).
            streaming: false,
            auto_decode: true,
            max_bytes: None,
        };

        let (tx_done, rx_done) = oneshot::channel::<NavigationResult<C>>();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let cookie_jar = self.services.cookie_jar.clone();

        let span = tracing::info_span!(
            "tab_nav",
            tab_id=%tab_id,
            nav_id=%nav_id.0,
            scheme=%url.scheme(),
            host=%url.host_str().unwrap_or(""),
            path=%url.path(),
        );

        let parent_cancel_clone = parent_cancel.clone();

        // Spawn the actual fetcher into a seperate task
        spawn_named("tab-fetcher", async move {
            let _enter = span.enter();

            let submit = submit_to_io(zone_id, req.clone(), io_tx.clone(), Some(parent_cancel_clone.clone())).await;

            let (handle, rx) = match submit {
                Ok(ok) => ok,
                Err(_) => {
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::NetworkError("I/O channel closed".into()),
                    });
                    return;
                }
            };

            let fetch_result: FetchResult = tokio::select! {
                _ = parent_cancel_clone.cancelled() => {
                    handle.cancel.cancel();
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Cancelled("Response channel closed".into())
                    });
                    return;
                }
                r = rx => match r {
                    Ok(r) => r,
                    Err(_) => {
                        let _ = tx_done.send(NavigationResult::Err {
                            nav_id,
                            error: NavigationError::Cancelled("Response channel closed".into())
                        });
                        return;
                    }
                }
            };

            // Store Set-Cookie headers from the navigation response.
            if let Some(meta) = fetch_result.meta() {
                cookie_jar
                    .write()
                    .store_response_cookies(&meta.final_url, &meta.headers, Some(&url));
            }

            let ua_policy = UaPolicy {
                enable_sniffing: false,
                enable_sniffing_navigation_upgrade: false,
                enable_pdf_viewer: false,
                render_unknown_text_in_tab: false,
                allow_download_without_user_activation: false,
            };

            let mut hooks = ResourcePipelines::<C>::new(zone_id, io_tx.clone());

            let outcome = route_response_for(
                RequestDestination::Document,
                handle,
                req.clone(),
                fetch_result.clone(),
                &ua_policy,
                &mut hooks,
            )
            .await;

            match outcome {
                Ok(RoutedOutcome::MainDocument(doc)) => {
                    use gosub_interface::document::Document as _;
                    let final_url = doc.url().unwrap_or_else(about_blank);
                    let title = crate::html::document_title(&doc);
                    let _ = tx_done.send(NavigationResult::Ok {
                        nav_id,
                        final_url,
                        title,
                        doc,
                    });
                }
                Ok(RoutedOutcome::ViewerRendered(_doc)) => {
                    log::warn!("Tab[{:?}] viewer rendering not supported yet", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Viewer rendering not supported yet")),
                    });
                }
                Ok(RoutedOutcome::DownloadStarted(_doc)) => {
                    log::warn!("Tab[{:?}] downloads not supported yet", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Download not supported yet")),
                    });
                }
                Ok(RoutedOutcome::DownloadFinished(_doc)) => {
                    log::warn!("Tab[{:?}] downloads not supported yet", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Download not supported yet")),
                    });
                }
                Ok(RoutedOutcome::CssLoaded(_doc)) => {
                    // CSS loaded, but we don't do anything special here
                    log::trace!("Tab[{:?}] RoutedOutcome::CssLoaded", tab_id);
                }
                Ok(RoutedOutcome::ScriptExecuted(_doc)) => {
                    // JS executed, but we don't do anything special here
                    log::trace!("Tab[{:?}] RoutedOutcome::ScriptExecuted", tab_id);
                }
                Ok(RoutedOutcome::ImageDecoded(_doc)) => {
                    // Image decoded, but we don't do anything special here
                    log::trace!("Tab[{:?}] RoutedOutcome::ImageDecoded", tab_id);
                }
                Ok(RoutedOutcome::FontLoaded(_doc)) => {
                    // Font loaded, but we don't do anything special here
                    log::trace!("Tab[{:?}] RoutedOutcome::FontLoaded", tab_id);
                }
                Ok(RoutedOutcome::Blocked(reason)) => {
                    log::debug!("Tab[{:?}] RoutedOutcome::Blocked", tab_id);

                    let final_url = match fetch_result.meta() {
                        Some(meta) => meta.final_url.clone(),
                        None => url.clone(),
                    };

                    _ = event_tx.send(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Failed {
                            nav_id: Some(nav_id),
                            url: final_url.clone(),
                            error: Arc::new(anyhow!("Reason: {}", reason)),
                        },
                    });
                }
                Ok(RoutedOutcome::Cancelled) => {
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Cancelled("Navigation cancelled".into()),
                    });
                }
                Err(e) => {
                    let err = format!("Routing error: {e}");
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::NetworkError(err),
                    });
                }
            }
        });

        self.load = Some(NavJoin {
            cancel: parent_cancel.clone(),
            rx: Some(rx_done),
        });
    }

    /// Do a draw tick. This will be called based on the FPS that is requested
    #[allow(unreachable_code)] // cfg-conditional tile-cache returns make the display-list path unreachable for some feature combos
    async fn tick_draw(&mut self) -> anyhow::Result<()> {
        // Skip rendering when nothing has changed to avoid burning CPU at the tick rate.
        if !self.runtime.dirty {
            return Ok(());
        }
        self.runtime.dirty = false;

        let render_backend = self.zone_context.render_backend.clone();

        // Install the active backend's rasterizer once (replaces the former per-backend cfg
        // selection and the Vello-specific wgpu_resources extraction).
        if !self.context.has_rasterizer() {
            // `create_rasterizer` is type-erased (the backend trait lives in `gosub_interface`,
            // which can't name the pipeline's `Rasterable`); recover it here.
            if let Some(rasterizer) =
                gosub_render_pipeline::rasterizer::downcast_rasterizer(render_backend.create_rasterizer())
            {
                self.context
                    .set_rasterizer(rasterizer, render_backend.raster_strategy());
            }
        }

        // TileCache path — used by every rasterizing backend (Cairo, Skia, Vello).
        //
        // These backends don't need the display-list render pipeline: tiles are rasterized
        // during stages 1-6 and the host composites them directly. A scroll-only fast path
        // skips stages 1-6 when only the offset changed.
        //
        // DPR comes from the backend: Cairo rasterizes at physical pixels (DPR > 1 on HiDPI);
        // Skia and Vello rasterize at CSS pixels (DPR = 1).
        if render_backend.raster_strategy() != RasterStrategy::None {
            let dpr = render_backend.device_pixel_ratio();

            // Scroll-only fast path: tiles are still valid, only the offset changed.
            if let Some(handle) = self.context.take_scroll_handle(dpr) {
                self.runtime.committed_scene_epoch = self.context.scene_epoch();
                self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
                return Ok(());
            }

            // Full render: rebuild stages 1-6 only (no display list), then submit TileCache.
            self.context.set_viewport(self.desired_viewport);
            self.context.rebuild_pipeline_cache_if_needed();
            let scene_epoch = self.context.scene_epoch();
            if let Some(handle) = self.context.tile_cache_handle(dpr) {
                self.runtime.committed_scene_epoch = scene_epoch;
                self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
            }
            self.sink.inc_frame();
            return Ok(());
        }

        // Null backend (no rasterizer): fall through to the display-list render path below.

        // Ensure we have a surface of the right size to draw on.
        // Track whether the surface was recreated (meaning pixels are blank and must be re-rendered).
        let surface_recreated = self.ensure_surface_tracked(render_backend.clone(), self.desired_viewport.as_size())?;
        // Propagate the current viewport so the pipeline lays out at the right dimensions.
        self.context.set_viewport(self.desired_viewport);
        // Rebuild the render list if anything has changed
        self.context.rebuild_render_list_if_needed();

        // Skip the expensive render+copy when neither the scene nor the surface changed.
        let scene_epoch = self.context.scene_epoch();
        if !surface_recreated && scene_epoch == self.runtime.committed_scene_epoch {
            return Ok(());
        }

        log::debug!(
            "[tick_draw] tab={:?} vp={}x{} render_items={} epoch={}",
            self.tab_id,
            self.desired_viewport.width,
            self.desired_viewport.height,
            self.context.render_list().items.len(),
            scene_epoch,
        );

        // Begin the render process
        let render_start = std::time::Instant::now();
        if let Some(ref mut surf) = self.surface {
            render_backend.render(&mut self.context, surf.as_mut())?;
            match render_backend.external_handle(surf.as_mut()) {
                Ok(handle) => {
                    log::debug!(
                        "[tick_draw] submitting handle: {}",
                        match &handle {
                            gosub_render_pipeline::render::backend::ExternalHandle::NullHandle {
                                width,
                                height,
                                ..
                            } => format!("NullHandle({}x{})", width, height),
                            gosub_render_pipeline::render::backend::ExternalHandle::CpuPixelsOwned {
                                width,
                                height,
                                stride,
                                pixels,
                                ..
                            } => format!(
                                "CpuPixelsOwned({}x{} stride={} bytes={})",
                                width,
                                height,
                                stride,
                                pixels.len()
                            ),
                            gosub_render_pipeline::render::backend::ExternalHandle::CpuPixelsPtr {
                                width,
                                height,
                                stride,
                                ..
                            } => format!("CpuPixelsPtr({}x{} stride={})", width, height, stride),
                            _ => "Other".to_string(),
                        }
                    );
                    self.runtime.committed_scene_epoch = scene_epoch;
                    let mut compositor = self.zone_context.compositor.write();
                    compositor.submit_frame(self.tab_id, handle);
                }
                Err(e) => {
                    log::warn!("[tick_draw] external_handle error: {e}");
                }
            }
        }
        let render_ms = render_start.elapsed().as_millis();

        self.sink.inc_frame();

        let now = std::time::Instant::now();
        let elapsed = now - self.runtime.last_tick_draw;
        self.runtime.last_tick_draw = now;

        // Convert to FPS
        if elapsed.as_secs_f32() > 0.0 {
            let fps = 1.0 / elapsed.as_secs_f32();
            self.sink.set_fps(fps);
            log::debug!("[render] frame {}ms  ({:.1} fps)", render_ms, fps);
        };

        Ok(())
    }

    /// Set a new viewport and schedule a re-render by transitioning to [`TabState::PendingRendering`].
    pub fn set_viewport(&mut self, vp: Viewport) {
        // Already at the viewport we want, then we can skip
        if vp == self.desired_viewport {
            return;
        }
        self.desired_viewport = vp;
        self.state = TabState::PendingRendering(self.desired_viewport);
        self.runtime.dirty = true;
    }

    /// Get the current snapshot image of the tab.
    pub fn thumbnail(&self) -> Option<&RgbaImage> {
        self.thumbnail.as_ref()
    }

    /// Bind local+session storage handles into the underlying browsing context.
    /// Call this after creating the tab or when the zone’s storage changes.
    pub fn bind_storage(&mut self, storage: StorageHandles) {
        self.context.bind_storage(storage.local, storage.session);
    }

    /// Dispatch a storage event to same-origin documents in this tab (placeholder).
    /// Intended for HTML5 storage event semantics.
    #[allow(dead_code)] // placeholder API, not wired into the storage layer yet
    pub(crate) fn dispatch_storage_events(&mut self, origin: &url::Origin, include_iframes: bool, ev: &StorageEvent) {
        log::trace!(
            "Tab[{:?}] dispatch_storage_events: origin={:?} include_iframes={} ev={:?}",
            self.tab_id,
            origin,
            include_iframes,
            ev
        );

        // Pseudocode stuff. need to fill in what it actually needs to do
        // for doc in self.iter_documents(include_iframes) {
        //     if doc.origin() == origin {
        //         // Don’t fire the event at the *mutating document* itself.
        //         if Some(self.id) == ev.source_tab && doc.is_the_mutating_document() {
        //             continue;
        //         }
        //         doc.A().dispatch_storage_event(
        //             ev.key.as_deref(),
        //             ev.old_value.as_deref(),
        //             ev.new_value.as_deref(),
        //             doc.url().to_string(),
        //             match ev.scope { StorageScope::Local => "local", StorageScope::Session => "session" }
        //         );
        //     }
        // }
    }

    /// Ensure the tab has a surface of the given size, creating it if necessary.
    /// Returns `true` when the surface was (re)created, meaning previously rendered
    /// pixels are gone and a full re-render is required even when the scene epoch
    /// hasn't changed.
    fn ensure_surface_tracked(
        &mut self,
        backend: Arc<dyn RenderBackend + Send + Sync>,
        size: SurfaceSize,
    ) -> anyhow::Result<bool> {
        if let Some(ref surf) = self.surface {
            if surf.size() == size {
                return Ok(false);
            }
        }
        self.surface = Some(backend.create_surface(size, self.present_mode)?);
        Ok(true)
    }

    /// Cancel the current navigation (if any)
    fn cancel_current_nav(&mut self) {
        if let Some(active) = self.active_nav.take() {
            log::warn!(
                "**** Cancelling active navigation for tab {:?} nav {:?}",
                self.tab_id,
                active.nav_id
            );
            active.cancel.cancel();
        }
    }

    /// Convert the URL string into an actual URL
    fn parse_url(&self, url: impl Into<String>) -> anyhow::Result<Url> {
        let unvalidated_url = url.into();

        match Url::parse(&unvalidated_url) {
            Ok(u) => Ok(u),
            Err(e) => {
                log::error!("Tab[{:?}]: Cannot parse URL: {}", self.tab_id, e);

                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::FailedUrl {
                        nav_id: None,
                        url: unvalidated_url.to_string(),
                        error: Arc::new(e.into()),
                    },
                });

                Err(NetError::Other(Arc::new(anyhow!("Cannot parse URL: {}", e))).into())
            }
        }
    }

    // Prepare storage for the URL
    fn bind_storage_for(&mut self, url: Url) -> anyhow::Result<()> {
        match self.prepare_storage_for(&url) {
            Ok(_) => Ok(()),
            Err(e) => {
                log::error!("Tab[{:?}]: Cannot prepare storage for URL {}: {}", self.tab_id, url, e);

                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::Failed {
                        nav_id: None,
                        url: url.clone(),
                        error: Arc::new(e),
                    },
                });

                Err(NetError::Other(Arc::new(anyhow!(
                    "Cannot bind storage for URL {}: {}",
                    self.tab_id,
                    url
                )))
                .into())
            }
        }
    }

    fn prepare_storage_for(&mut self, url: &Url) -> anyhow::Result<()> {
        let pk = compute_partition_key(url, self.services.partition_policy);
        let origin = url.origin().clone();

        let local = self
            .services
            .storage
            .local_for(self.zone_id, &pk, &origin)
            .context("cannot get local storage for tab")?;

        let session = self
            .services
            .storage
            .session_for(self.zone_id, self.tab_id, &pk, &origin)
            .context("cannot get session storage for tab")?;

        self.bind_storage(StorageHandles { local, session });
        Ok(())
    }
}

enum ControlFlow {
    Continue,
    Break,
}

impl ControlFlow {
    fn is_break(&self) -> bool {
        matches!(self, ControlFlow::Break)
    }
}

#[cfg(test)]
mod tests {
    use crate::net::SharedBody;
    use bytes::Bytes;
    use futures_util::TryStreamExt;

    #[tokio::test]
    async fn shared_body_streamreader_eof() {
        use std::io;
        use tokio::io::AsyncReadExt;
        use tokio_util::io::StreamReader;

        let sb = SharedBody::new(16);

        // Consumer
        let mut reader = StreamReader::new(sb.subscribe_stream().map_err(io::Error::other));

        // Producer
        sb.push(Bytes::from_static(&[0u8; 8192]));
        sb.push(Bytes::from_static(&[0u8; 8192]));
        sb.push(Bytes::from_static(&[0u8; 8192]));
        sb.push(Bytes::from_static(&[0u8; 8192]));
        sb.push(Bytes::from_static(&[0u8; 1948]));
        sb.finish();

        // Drain all
        let mut total = 0usize;
        let mut buf = [0u8; 4096];
        loop {
            let n = reader.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(total, 4 * 8192 + 1948);
    }
}
