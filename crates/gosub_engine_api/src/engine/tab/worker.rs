use crate::engine::errors::NavigationError;
use crate::engine::events::{EngineEvent, NavigationEvent, TabInternalCommand};
use crate::engine::pipeline::Hooks;
use crate::engine::types::{NavigationId, RequestId};
use crate::engine::{BrowsingContext, UaPolicy};
use crate::events::{IoCommand, TabCommand};
use crate::net::types::{
    FetchKeyData, FetchRequest, FetchResult, Initiator, NetError, Priority, ResourceKind,
};
use crate::net::{route_response_for, submit_to_io, RequestDestination, RoutedOutcome};
use crate::render::backend::{ErasedSurface, PresentMode, RenderBackend, RgbaImage, SurfaceSize};
use crate::render::{DevicePixelRatio, Viewport};
use crate::storage::types::compute_partition_key;
use crate::storage::{StorageEvent, StorageHandles};
use crate::tab::services::EffectiveTabServices;
use crate::tab::state::{TabActivityMode, TabRuntime, TabState};
use crate::tab::{TabId, TabSink};
use crate::util::spawn_named;
use crate::zone::{ZoneContext, ZoneId};
use anyhow::{anyhow, Context};
use http::Method;
use std::sync::Arc;
use tokio::select;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use url::Url;
use crate::net::req_ref_tracker::RequestReference;

#[derive(Debug)]
pub enum NavigationResult {
    Ok {
        nav_id: NavigationId,
        final_url: Url,
        title: Option<String>,
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

struct NavJoin {
    // nav_id: NavigationId,
    cancel: CancellationToken,
    rx: oneshot::Receiver<NavigationResult>,
}

pub struct TabWorker {
    /// ID of the tab
    pub tab_id: TabId,
    /// ID of the zone in which this tab resides
    pub zone_id: ZoneId,

    /// Shared context from the tab
    zone_context: Arc<ZoneContext>,
    // Effective tab services that we can use
    services: EffectiveTabServices,

    /// Sink for sending events upwards
    sink: Arc<TabSink>,

    /// Receiver for incoming tab commands
    cmd_rx: mpsc::Receiver<TabCommand>,

    /// Browsing context running for this tab
    pub context: BrowsingContext,
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
    #[allow(unused)]
    surface: Option<Box<dyn ErasedSurface + Send>>,
    // // Size of the surface (does not have to match viewport)
    // surface_size: SurfaceSize,
    // Present mode for the surface?
    #[allow(unused)]
    present_mode: PresentMode,
    /// Device Pixel Ratio
    #[allow(unused)]
    dpr: DevicePixelRatio,
    /// The viewport that was committed for the in-flight/last render
    #[allow(unused)]
    committed_viewport: Viewport,
    /// The newest viewport requested by the tab, which may differ from the committed one.
    desired_viewport: Viewport,
    /// Set when a resize arrives while rendering. Causes an immediate re-render after finishing the current rendering.
    dirty_after_inflight: bool,
    /// Keeps track of the tab worker runtime data
    pub(crate) runtime: TabRuntime,
    /// Current in-flight navigation (if any)
    load: Option<NavJoin>,
    /// Current active navigation (if any)
    active_nav: Option<ActiveNav>,

    // Send channel for internal commands
    internal_tx: mpsc::Sender<TabInternalCommand>,
    // Receive channel for internal commands
    internal_rx: mpsc::Receiver<TabInternalCommand>,
}

impl TabWorker {
    /// Creates a new tab. Does NOT spawn the tab worker
    pub fn new(
        tab_id: TabId,
        zone_id: ZoneId,
        services: EffectiveTabServices,
        zone_context: Arc<ZoneContext>,
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
            dpr: DevicePixelRatio(1.0),
            committed_viewport: Default::default(),
            desired_viewport: Default::default(),
            dirty_after_inflight: false,
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

        loop {
            select! {
                // Handle tick for redraws
                _ = self.runtime.interval.tick(), if self.runtime.drawing_enabled => {
                    if let Err(e) = self.tick_draw().await {
                        self.state = TabState::Failed(format!("Tab {:?} tick error: {}", self.tab_id, e));
                        self.runtime.dirty = true;
                    }
                }

                // In-flight load completion
                result = async {
                    // Wait until the self.runtime.load.rx channel (if any) resolves
                    let load = self.load.take().expect("select! branch is guarded by is_some()");
                    load.rx.await
                }, if self.load.is_some() => {
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
                }
            }
        }

        self.send_event(EngineEvent::TabClosed {
            tab_id: self.tab_id,
            zone_id: self.zone_id,
        });
        self.services.storage.drop_tab(self.zone_id, self.tab_id);
    }

    fn on_nav_result(&mut self, res: NavigationResult) {
        match res {
            NavigationResult::Ok {
                nav_id,
                final_url,
                title,
            } => {
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
                    .unwrap_or_else(|| Url::parse("about:blank").unwrap());

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

    fn handle_internal_command(&mut self, cmd: TabInternalCommand) {
        match cmd {
            TabInternalCommand::SetDocument { doc } => {
                self.context.set_document(doc);
                self.runtime.dirty = true;
            }
        }
    }

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
            TabCommand::SetViewport { x, y, width, height } => {
                self.set_viewport(Viewport::new(x, y, width, height));
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::MouseMove { .. }
            | TabCommand::MouseDown { .. }
            | TabCommand::MouseUp { .. }
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
                log::warn!(
                    "{}",
                    format!(
                        "Tab {:?} received unhandled command: {:?}",
                        self.tab_id, cmd
                    )
                );
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
            let mut guard = self.zone_context.request_reference_map.write().unwrap();
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

        let req = FetchRequest {
            reference: RequestReference::Navigation(nav_id),
            req_id: RequestId::new(),
            key_data: FetchKeyData {
                url: url.clone(),
                method: Method::GET,
                headers: Default::default(),
            },
            priority: Priority::High,
            kind: ResourceKind::Document,
            initiator: Initiator::Navigation,
            streaming: true,
            auto_decode: true,
            max_bytes: None,
        };

        let (tx_done, rx_done) = oneshot::channel::<NavigationResult>();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let internal_tx = self.internal_tx.clone();

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
        tokio::spawn(async move {
            let _enter = span.enter();

            let submit = submit_to_io(
                zone_id,
                req.clone(),
                io_tx.clone(),
                Some(parent_cancel_clone.clone()),
            )
            .await;

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

            let ua_policy = UaPolicy {
                enable_sniffing: false,
                enable_sniffing_navigation_upgrade: false,
                enable_pdf_viewer: false,
                render_unknown_text_in_tab: false,
                allow_download_without_user_activation: false,
            };

            let mut hooks = Hooks::new(zone_id, io_tx.clone());

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

                    // Update our document in the tab
                    internal_tx.send(TabInternalCommand::SetDocument { doc: doc.clone() }).await.ok();

                    let _ = tx_done.send(NavigationResult::Ok {
                        nav_id,
                        final_url: doc.final_url.clone(),
                        title: doc.title.clone(),
                    });
                    return;
                }
                Ok(RoutedOutcome::ViewerRendered(_doc)) => {
                    println!("Tab[{:?}] RoutedOutcome::ViewerRendered", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Viewer rendering not supported yet")),
                    });
                    return;
                }
                Ok(RoutedOutcome::DownloadStarted(_doc)) => {
                    println!("Tab[{:?}] RoutedOutcome::DownloadStarted", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Download not supported yet")),
                    });
                    return;
                }
                Ok(RoutedOutcome::DownloadFinished(_doc)) => {
                    println!("Tab[{:?}] RoutedOutcome::DownloadFinished", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Download not supported yet")),
                    });
                    return;
                }
                Ok(RoutedOutcome::CssLoaded(_doc)) => {
                    // CSS loaded, but we don't do anything special here
                    println!("Tab[{:?}] RoutedOutcome::CssLoaded", tab_id);
                }
                Ok(RoutedOutcome::ScriptExecuted(_doc)) => {
                    // JS executed, but we don't do anything special here
                    println!("Tab[{:?}] RoutedOutcome::ScriptExecuted", tab_id);
                }
                Ok(RoutedOutcome::ImageDecoded(_doc)) => {
                    // Image decoded, but we don't do anything special here
                    println!("Tab[{:?}] RoutedOutcome::ImageDecoded", tab_id);
                }
                Ok(RoutedOutcome::FontLoaded(_doc)) => {
                    // Font loaded, but we don't do anything special here
                    println!("Tab[{:?}] RoutedOutcome::FontLoaded", tab_id);
                }
                Ok(RoutedOutcome::Blocked(reason)) => {
                    println!("Tab[{:?}] RoutedOutcome::Blocked", tab_id);

                    let final_url = match fetch_result.meta() {
                        Some(meta) => meta.final_url.clone(),
                        None => url.clone(),
                    };

                    _ = event_tx.send(EngineEvent::Navigation {
                        tab_id,
                        event: NavigationEvent::Failed {
                            nav_id: Some(nav_id),
                            url: final_url.clone(),
                            error: Arc::new(anyhow!("Reason: {}", reason).into()),
                        },
                    });
                }
                Ok(RoutedOutcome::Cancelled) => {
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Cancelled("Navigation cancelled".into()),
                    });
                    return;
                }
                Err(e) => {
                    let err = format!("Routing error: {}", e.to_string());
                    let _ = tx_done.send(NavigationResult::Err{nav_id, error: NavigationError::NetworkError(err) });
                    return;
                }
            }
        });

        self.load = Some(NavJoin {
            cancel: parent_cancel.clone(),
            rx: rx_done,
        });
    }

    /// Do a draw tick. This will be called based on the FPS that is requested
    async fn tick_draw(&mut self) -> anyhow::Result<()> {
        let render_backend = self.zone_context.render_backend.clone();

        // Ensure we have a surface of the right size to draw on
        self.ensure_surface(render_backend.clone(), self.desired_viewport.as_size())?;
        // Rebuild the render list if anything has changed
        self.context.rebuild_render_list_if_needed();

        // Begin the render process
        if let Some(ref mut surf) = self.surface {
            render_backend.render(&mut self.context, surf.as_mut())?;
            if let Ok(handle) = render_backend.external_handle(surf.as_mut()) {
                let mut compositor = self.zone_context.compositor.write().unwrap();
                compositor.submit_frame(self.tab_id, handle);
            }
        }

        self.sink.inc_frame();

        let now = std::time::Instant::now();
        let elapsed = now - self.runtime.last_tick_draw;
        self.runtime.last_tick_draw = now;

        // Convert to FPS
        if elapsed.as_secs_f32() > 0.0 {
            let fps = 1.0 / elapsed.as_secs_f32();
            self.sink.set_fps(fps);
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

        if matches!(self.state, TabState::Rendering(_)) {
            // We are currently rending, so we can cancel the current rendering
            self.dirty_after_inflight = true;
        } else {
            // Start rendering with the new viewport
            self.state = TabState::PendingRendering(self.desired_viewport)
        }

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
    #[allow(unused)]
    pub(crate) fn dispatch_storage_events(&mut self, origin: &url::Origin, include_iframes: bool, ev: &StorageEvent) {
        println!("Tab {:?} dispatch_storage_events called", self.tab_id);
        dbg!(&origin);
        dbg!(&include_iframes);
        dbg!(&ev);

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
    #[allow(unused)]
    fn ensure_surface(&mut self, backend: Arc<dyn RenderBackend + Send + Sync>, size: SurfaceSize) -> anyhow::Result<()> {
        if let Some(ref surf) = self.surface {
            if surf.size() == size {
                return Ok(());
            }
        }
        self.surface = Some(backend.create_surface(size, self.present_mode)?);
        Ok(())
    }

    #[allow(unused)]
    fn begin_render(&mut self, render_backend: Arc<dyn RenderBackend + Send + Sync>) -> anyhow::Result<()> {
        if self.committed_viewport != self.desired_viewport {
            self.committed_viewport = self.desired_viewport;

            let surf_sz = self.committed_viewport.to_surface_size(self.dpr);
            self.ensure_surface(render_backend, surf_sz)?;
            self.context.set_viewport(self.committed_viewport);
        }

        Ok(())
    }

    #[allow(unused)]
    fn end_render(&mut self) {
        if self.dirty_after_inflight {
            self.dirty_after_inflight = false;
            self.state = TabState::PendingRendering(self.desired_viewport);
            self.runtime.dirty = true;
        } else {
            self.state = TabState::Idle;
        }
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
                log::error!(
                    "Tab[{:?}]: Cannot prepare storage for URL {}: {}",
                    self.tab_id,
                    url,
                    e
                );

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

///
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
        let mut reader = StreamReader::new(
            sb.subscribe_stream()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e)),
        );

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
