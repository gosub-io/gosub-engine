use crate::engine::errors::{LoadError, NavigationError};
use crate::engine::events::IoCommand;
use crate::engine::events::Modifiers;
use crate::engine::events::{CursorShape, EngineEvent, NavigationEvent};
use crate::engine::events::{DownloadOfferId, PendingDownload};
use crate::engine::internal_pages::{InternalPages, TabView};
use crate::engine::resource_pipeline::ResourcePipelines;
use crate::engine::types::{NavigationId, RequestId};
use crate::engine::{BrowsingContext, UaPolicy};
use crate::events::TabCommand;
use crate::fork_server::protocol::InputEvent;
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{
    FetchHandle, FetchRequest, FetchResult, FetchResultMeta, Initiator, NetError, Priority, RequestBody, ResourceKind,
};
use crate::net::{route_response_for, submit_to_io, RequestDestination, RoutedOutcome};
use crate::storage::types::compute_partition_key;
use crate::storage::StorageHandles;
use crate::tab::history::{History, HistoryEntryId};
use crate::tab::scroll::{default_text_scroll, ScrollState};
use crate::tab::services::EffectiveTabServices;
use crate::tab::state::{TabRuntime, TabState};
use crate::tab::{TabId, TabSink};
use crate::util::spawn_named;
use crate::zone::{ZoneContext, ZoneId};
use anyhow::{anyhow, Context};
use gosub_render_pipeline::rasterizer::RasterStrategy;
use gosub_render_pipeline::render::backend::{
    CompositorSink, ErasedSurface, ExternalHandle, PresentMode, RenderBackend, SurfaceSize,
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

/// An icon larger than this is not an icon.
const MAX_FAVICON_BYTES: usize = 512 * 1024;

/// Move an accepted offer's spooled body to `target_path`, returning the bytes written.
/// Blocking; callers run it on the blocking pool.
///
/// `persist` renames first (same filesystem, no copy) and falls back to a copy across mount
/// points - the common case when the temp dir and the download directory differ.
fn place_spooled_download(spooled: tempfile::TempPath, target_path: &std::path::Path) -> anyhow::Result<u64> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if let Err(e) = spooled.persist(target_path) {
        std::fs::copy(&e.path, target_path).with_context(|| format!("copy to {}", target_path.display()))?;
        // `e.path` is still a TempPath, so the source is removed when it drops.
    }
    Ok(std::fs::metadata(target_path).map(|m| m.len()).unwrap_or(0))
}

/// A download offer whose body is on disk, awaiting the embedder's answer.
struct PendingOffer {
    info: PendingDownload,
    body: tempfile::TempPath,
}

/// Where a locally loaded document's HTML comes from.
enum HtmlSource {
    Text(String),
    /// A spooled download body, read (bounded) on the load task rather than the worker.
    Spooled(tempfile::TempPath),
}

/// Stream a response body to `path`, emitting `DownloadProgress` roughly every 256 KiB
/// and `DownloadFinished` once the file is fully written.
async fn stream_to_file(
    id: crate::engine::events::DownloadId,
    tab_id: TabId,
    event_tx: &crate::engine::types::EventChannel,
    total_bytes: Option<u64>,
    peek_buf: crate::engine::types::PeekBuf,
    shared: Arc<crate::net::SharedBody>,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const REPORT_EVERY: u64 = 256 * 1024;
    let mut reader = crate::net::SharedBody::combined_reader(peek_buf, shared);
    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("create {}", path.display()))?;
    let mut received: u64 = 0;
    let mut last_reported: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).await.context("read body")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .await
            .with_context(|| format!("write {}", path.display()))?;
        received += n as u64;
        if received - last_reported >= REPORT_EVERY {
            last_reported = received;
            let _ = event_tx.send(EngineEvent::DownloadProgress {
                tab_id,
                id,
                received_bytes: received,
                total_bytes,
            });
        }
    }
    file.flush().await.context("flush")?;
    let _ = event_tx.send(EngineEvent::DownloadFinished {
        tab_id,
        id,
        path: path.to_path_buf(),
        received_bytes: received,
    });
    Ok(())
}

/// Minimal scope guard: runs `f` on drop. Used where a task must clean up on every exit
/// path without pulling in a dependency.
fn scopeguard<F: FnMut()>(f: F) -> impl Drop {
    struct Guard<F: FnMut()>(F);
    impl<F: FnMut()> Drop for Guard<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    Guard(f)
}

/// Fallback URL used when a navigation has no usable URL.
///
/// The one `unwrap` left in this crate, and deliberately: both callers need a concrete `Url` for
/// the navigation event API, so making this fallible would push an `Option` out through
/// `NavigationEvent` and `NavigationResult` to every consumer of a navigation event - a long way
/// to carry a case that cannot happen. `url` offers no infallible constructor, so parsing is the
/// only way to build one.
///
/// What makes "cannot happen" more than a comment is `about_blank_parses` below, which is the
/// test the old `// PANIC-SAFE: literal URL` was implicitly claiming existed.
#[allow(clippy::unwrap_used)]
fn about_blank() -> Url {
    Url::parse("about:blank").unwrap()
}

#[derive(Debug)]
pub enum NavigationResult<C: RenderConfiguration> {
    Ok {
        nav_id: NavigationId,
        final_url: Url,
        title: Option<String>,
        /// `None` when a renderer process parses the document instead.
        doc: Option<Arc<crate::html::EngineDocument<C>>>,
        /// The document's source text, captured when this engine renders
        /// out-of-process (the renderer re-parses it there).
        source: Option<Arc<str>>,
    },
    Err {
        nav_id: NavigationId,
        error: NavigationError,
    },
    /// The response is non-renderable content: the navigation ends (page stays) and the
    /// metadata becomes a `DownloadRequested` offer to the embedder; `spooled` holds the
    /// body already fetched.
    Download {
        nav_id: NavigationId,
        meta: FetchResultMeta,
        spooled: tempfile::TempPath,
    },
}

// Current active navigation
struct ActiveNav {
    pub nav_id: NavigationId,
    pub cancel: CancellationToken,
    pub url: Url,
    /// How this navigation relates to session history, decided when it starts and applied
    /// when it commits (see `on_nav_result`).
    pub history: HistoryIntent,
}

/// What a navigation does to the tab's session history once it commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryIntent {
    /// A fresh navigation (URL bar, link click, LoadHtml): push a new entry.
    Push,
    /// Reload: keep the current entry, refresh its URL, restore its scroll offset.
    Reload,
    /// Back/forward/jump: the cursor already moved to `entry` when the navigation started;
    /// on commit only the entry's saved scroll offset is restored.
    Traverse(HistoryEntryId),
}

struct NavJoin<C: RenderConfiguration> {
    cancel: CancellationToken,
    // Wrapped in Option so the receiver can be extracted into `pending_nav_rx`
    // without dropping the cancel token from self.load.
    rx: Option<oneshot::Receiver<NavigationResult<C>>>,
}

pub struct TabWorker<C: RenderConfiguration> {
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

    // Surface on which the browsing context can render the tab
    surface: Option<Box<dyn ErasedSurface + Send>>,
    // Present mode for the surface?
    present_mode: PresentMode,
    /// The newest viewport requested by the tab, which may differ from the committed one.
    desired_viewport: Viewport,
    /// Current scroll offset in CSS pixels (updated by MouseScroll). Mirrors the integer-rounded
    /// position held by `scroll`; the rest of the worker reads these.
    scroll_x: i32,
    scroll_y: i32,
    /// Engine-side scroll position + smooth-scroll animation. Defaults to `Instant`, so behaviour is
    /// unchanged until the engine takes scrolling over from the embedder (see [`ScrollBehavior`]).
    scroll: ScrollState,
    /// Timestamp of the last scroll-animation step, for computing `dt`. `None` when not animating.
    scroll_anim_last: Option<std::time::Instant>,
    /// Last cursor shape sent to the embedder, so moves only emit on change.
    /// Keeps track of the tab worker runtime data
    pub(crate) runtime: TabRuntime,
    /// Current in-flight navigation (if any)
    load: Option<NavJoin<C>>,
    /// Current active navigation (if any)
    active_nav: Option<ActiveNav>,
    /// Timing scope for the navigation currently being loaded or displayed.
    timing_scope: Option<gosub_shared::timing::ScopeId>,
    /// Whether `page.first_paint` has been marked for `timing_scope` yet. First paint is
    /// once per navigation, and `tick_draw` runs on every frame.
    first_paint_marked: bool,
    /// Session history (tree). Fresh navigations push, back/forward move the cursor.
    history: History,
    reported_cursor: CursorShape,
    /// A resident renderer holds the pointer (a drag, an open dropdown): moves
    /// and wheel go to it rather than through hover and page scrolling.
    remote_capture: bool,
    /// What a resident renderer last said about focus on its page: `Some(editable)`
    /// while something is focused. Decides which keys scroll the page here.
    remote_focus: Option<bool>,
    /// Scroll to apply once the just-committed document has laid out (positions and page
    /// height are only known then, and `set_scroll` clamps against the latter). Set by
    /// `on_nav_result`, consumed by `tick_draw`.
    pending_scroll: Option<PendingScroll>,
    /// Download offers whose body is still held, oldest first; looked up by
    /// [`DownloadOfferId`]. Bounded by `remember_offer`.
    pending_offers: std::collections::VecDeque<PendingOffer>,
    /// Source of [`DownloadOfferId`]s; unique within the tab.
    next_offer: u64,
    /// The icon last fetched for the document a renderer process reported, so
    /// the renders that follow do not fetch it again. Cleared when a navigation
    /// commits a new document.
    remote_favicon: Option<Url>,
    /// The history entry the shown document was committed to. A traversal
    /// moves the cursor before its load commits, and a title the renderer
    /// reports meanwhile is still this document's, not the cursor's.
    document_entry: Option<HistoryEntryId>,
}

/// Deferred scroll for a freshly committed document.
#[derive(Debug, Clone, PartialEq)]
enum PendingScroll {
    /// Restore a saved history offset (reload, back/forward).
    Offset(i32, i32),
    /// Scroll to the element the URL fragment indicates (fresh load of `…#anchor`).
    Fragment(String),
}

impl<C: RenderConfiguration> TabWorker<C> {
    /// Creates a new tab. Does NOT spawn the tab worker
    pub fn new(
        tab_id: TabId,
        zone_id: ZoneId,
        services: EffectiveTabServices,
        zone_context: Arc<ZoneContext<C>>,
        sink: Arc<TabSink>,
        cmd_rx: mpsc::Receiver<TabCommand>,
    ) -> Self {
        let config_store = zone_context.config_store.clone();
        #[allow(unused_mut)] // mut only used on the isolation-capable platform below
        let mut context = BrowsingContext::with_loader(
            config_store.clone(),
            crate::net::brokered_loader::BrokeredLoader::new(zone_id, Some(tab_id), zone_context.io_tx.clone())
                .with_accept_language(services.accept_language.clone())
                .shared(),
        );

        // Install this tab's remote-render mode per the configured font
        // system's (static) confinement tier: `Full` renders through the
        // engine's warmed fork server, `FontPathsReadable` spawns a throwaway
        // exec'd renderer per render, `Unsupported` stays in-process. A remote
        // render hands back CPU tiles for the host to composite, so a backend
        // that presents a GPU texture instead (Vello) stays in-process too.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        if zone_context.render_backend.renders_to_gpu_texture() {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                log::info!(
                    "renderer process off: the {} backend presents a GPU texture, and remote renders \
                     produce CPU tiles",
                    zone_context.render_backend.name()
                )
            });
        } else {
            use crate::engine::context::RemoteRenderer;
            use gosub_interface::font_system::{Confinement, FontSystem as _};
            match C::FontSystem::confinement() {
                Confinement::Full => {
                    if let Some(pool) = zone_context.engine_context.renderer_pool.get() {
                        context.set_remote_renderer(
                            RemoteRenderer::Resident {
                                pool: Arc::clone(pool),
                                zone: zone_id,
                                tab: tab_id,
                            },
                            tab_id.to_string(),
                        );
                    } else if let Some(server) = zone_context.engine_context.renderer_process.get() {
                        context.set_remote_renderer(RemoteRenderer::ForkServer(Arc::clone(server)), tab_id.to_string());
                    }
                }
                Confinement::FontPathsReadable => {
                    if config_store.get_bool("security.renderer_process") {
                        context.set_remote_renderer(RemoteRenderer::ExecPerRender, tab_id.to_string());
                    }
                }
                Confinement::Unsupported(_) => {}
            }
        }
        let runtime = TabRuntime::with_fps(config_store.get_uint("renderer.tab.default_fps") as u32);

        Self {
            tab_id,
            zone_id,
            services,
            zone_context,
            sink,
            cmd_rx,
            context,
            state: TabState::Idle,
            favicon: vec![],
            title: config_store.get_string("useragent.tab.default_title"),
            pending_url: None,
            current_url: None,
            is_loading: false,
            is_error: false,
            surface: None,
            present_mode: PresentMode::Fifo,
            desired_viewport: Default::default(),
            scroll_x: 0,
            scroll_y: 0,
            // The engine owns wheel-scroll smoothing; embedders send one delta per notch.
            scroll: ScrollState::new(default_text_scroll()),
            scroll_anim_last: None,
            runtime,
            load: None,
            active_nav: None,
            timing_scope: None,
            first_paint_marked: false,
            history: History::default(),
            reported_cursor: CursorShape::Default,
            remote_capture: false,
            remote_focus: None,
            pending_scroll: None,
            pending_offers: std::collections::VecDeque::new(),
            next_offer: 0,
            remote_favicon: None,
            document_entry: None,
        }
    }

    /// Spawns the tab worker into a new task and returns the join handle
    /// Give the media store somewhere to send the requests it cannot make itself.
    ///
    /// Here rather than in [`Self::new`], which is a synchronous public constructor: the
    /// source needs a runtime handle to spawn its fetches on -- its callers are plain threads
    /// waiting on the hand-off, with no runtime of their own -- and `Handle::current()`
    /// panics outside a runtime. `run_worker` is a future, so by definition something is
    /// polling it, and it runs before the first layout can ask for an image.
    fn wire_media_source(&mut self) {
        self.context.set_media_source(std::sync::Arc::new(
            crate::engine::media_source::EngineMediaSource::new(
                self.zone_id,
                self.tab_id,
                self.zone_context.io_tx.clone(),
                tokio::runtime::Handle::current(),
                self.services.accept_language.clone(),
            ),
        ));
    }

    pub fn spawn_worker(self) -> anyhow::Result<JoinHandle<()>> {
        let name = format!("Tab Worker {}", self.tab_id);
        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let event_tx = self.zone_context.event_tx.clone();
        let tab_identities = self.zone_context.tab_identities.clone();
        let worker = spawn_named(&name, self.run_worker());

        // Crash containment (in-process): a panic anywhere in the worker kills only its
        // task. This watchdog turns that into a `TabCrashed` event so the shell can show
        // a crashed-tab page and recreate the tab, instead of a silently dead handle.
        let join_handle = spawn_named(&format!("{name} watchdog"), async move {
            let Err(join_err) = worker.await else {
                return; // clean exit: TabClosed was emitted by the run loop
            };
            let error = if join_err.is_panic() {
                match join_err.into_panic().downcast::<String>() {
                    Ok(msg) => *msg,
                    Err(payload) => payload
                        .downcast::<&'static str>()
                        .map(|msg| msg.to_string())
                        .unwrap_or_else(|_| "panic with non-string payload".into()),
                }
            } else {
                "worker task was cancelled".into()
            };
            log::error!("Tab[{tab_id:?}] worker crashed: {error}");
            // The run loop's own cleanup never ran: drop the jar reference here,
            // so a fetch the dead tab left behind goes out without cookies.
            tab_identities.remove(tab_id);
            let _ = event_tx.send(EngineEvent::TabCrashed { tab_id, zone_id, error });
        });

        Ok(join_handle)
    }

    /// One frame onto the telemetry firehose: how it was produced and what it
    /// cost, so a viewer can see stalls as they happen.
    fn report_frame(&self, path: &str, started: std::time::Instant) {
        if !crate::telemetry::enabled() {
            return;
        }
        crate::telemetry::emit(
            "tab.frame",
            serde_json::json!({
                "tab": self.tab_id.to_string(),
                "path": path,
                "frame_us": started.elapsed().as_micros() as u64,
                "scroll_y": self.context.scroll_xy().1,
            }),
        );
    }

    // Main loop of the tab worker
    async fn run_worker(mut self) {
        self.sink.set_worker_started_now();
        self.wire_media_source();

        // Publish this tab's jar to the I/O side, which attaches cookies on its
        // behalf from now on - the tab itself never handles a cookie value.
        self.zone_context
            .tab_identities
            .register(self.tab_id, self.services.cookie_jar.clone());

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

                // In-flight load completion - uses a persistent receiver so it is not
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

        // Drop the jar reference before announcing closure: a fetch that outlives
        // the tab then goes out without cookies rather than against a stale jar.
        self.zone_context.tab_identities.remove(self.tab_id);
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.context.release_remote_renderer();

        // Receiver may already be gone at shutdown; that is expected.
        let _ = self.zone_context.event_tx.send(EngineEvent::TabClosed {
            tab_id: self.tab_id,
            zone_id: self.zone_id,
        });
        self.services.storage.drop_tab(self.zone_id, self.tab_id);
    }

    /// Fetch the document's icon through the zone fetcher (so it carries the UA, cookies and
    /// shows up in resource events) and emit `FavIconChanged` with its bytes on success.
    /// Fire-and-forget: runs on its own task, cancelled with the navigation.
    fn fetch_favicon(&self, icon_url: Url, nav_cancel: &CancellationToken) {
        let Some(base_url) = self.context.document_url().cloned() else {
            return;
        };
        // The icon URL is the page's (via the renderer): web schemes only,
        // plus a file page's own files.
        let allowed = matches!(icon_url.scheme(), "http" | "https")
            || (icon_url.scheme() == "file" && base_url.scheme() == "file");
        if !allowed {
            log::debug!("favicon {icon_url}: scheme not allowed, ignored");
            return;
        }
        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Image, Initiator::Other);
        let mut headers = HeaderMap::new();
        if let Ok(val) = ResourceKind::Image.accept_header().parse() {
            headers.insert(http::header::ACCEPT, val);
        }
        // The referrer marks the requesting document; it lets file:// pages load their
        // own icons (the file loader gates subresources on it).
        let req = FetchRequest::builder(Method::GET, icon_url.clone())
            .with_req_id(req_id)
            .with_headers(headers)
            .with_priority(Priority::Low)
            .with_kind(ResourceKind::Image.to_net())
            .with_initiator(Initiator::Other.to_net())
            .with_referrer(base_url.clone())
            .with_streaming(false)
            .with_auto_decode(true)
            .build();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let cancel = nav_cancel.child_token();
        spawn_named("tab-favicon", async move {
            let Ok((handle, rx)) = submit_to_io(zone_id, Some(tab_id), req, io_tx, Some(cancel.clone())).await else {
                return;
            };
            let result = tokio::select! {
                _ = cancel.cancelled() => {
                    handle.cancel.cancel();
                    return;
                }
                r = rx => r,
            };
            let Ok(FetchResult::Buffered { meta, body }) = result else {
                return;
            };
            if meta.status != 200 || body.is_empty() || body.len() > MAX_FAVICON_BYTES {
                log::debug!(
                    "favicon {icon_url}: status {} ({} bytes), ignored",
                    meta.status,
                    body.len()
                );
                return;
            }
            // The embedder will hand these bytes to an image decoder in its
            // own process: at least require the response to say it is an image.
            // A missing or unreadable type says nothing of the kind.
            let is_image = meta
                .headers
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| {
                    // Media types are case-insensitive: `Image/PNG` is an image.
                    ct.trim_start()
                        .get(..6)
                        .is_some_and(|t| t.eq_ignore_ascii_case("image/"))
                });
            if !is_image {
                log::debug!("favicon {icon_url}: not an image content type, ignored");
                return;
            }
            let _ = event_tx.send(EngineEvent::FavIconChanged {
                tab_id,
                favicon: body.to_vec(),
            });
        });
    }

    /// Whether this tab's full renders go out-of-process (fork server or
    /// exec-per-render). Decides both the routing and whether navigation
    /// captures the document source (the renderer re-parses it there).
    fn remote_render_available(&self) -> bool {
        // What `new` installed for this tab, not what the engine could offer:
        // a backend that presents a GPU texture gets no renderer although the
        // engine has one, and a tab without one must parse its documents here.
        self.context.has_remote_renderer()
    }

    fn on_nav_result(&mut self, res: NavigationResult<C>) {
        match res {
            NavigationResult::Ok {
                nav_id,
                final_url,
                title,
                doc,
                source,
            } => {
                // A new document: whatever icon the last one had is not this one's.
                self.remote_favicon = None;
                // Everything the pipeline records from here belongs to this navigation.
                // Set before the document so the first rebuild is already attributed.
                let scope = gosub_shared::timing::ScopeId(nav_id.0);
                self.context.set_timing_scope(Some(scope));
                self.timing_scope = Some(scope);
                // Explicit scope rather than the thread-local one: this is the worker's
                // async loop, where a thread-local scope is not reliable.
                gosub_shared::timing::mark_in(
                    scope,
                    gosub_shared::timing::Timing::PageDomComplete,
                    Some(final_url.to_string()),
                );
                self.context.set_media_navigation(
                    Some(final_url.clone()),
                    crate::net::req_ref_tracker::RequestReference::Navigation(nav_id),
                );
                let nav_cancel = self
                    .active_nav
                    .as_ref()
                    .filter(|a| a.nav_id == nav_id)
                    .map(|a| a.cancel.clone());
                match (doc, source) {
                    (Some(doc), source) => {
                        self.context.set_document(Arc::clone(&doc), source);
                        if let Some(cancel) = &nav_cancel {
                            if let Some(icon) = crate::html::favicon_url::<C>(&doc, &final_url) {
                                self.fetch_favicon(icon, cancel);
                            }
                        }
                    }
                    // The renderer process parses; title and icon arrive with
                    // its first render (see `apply_remote_document_meta`).
                    (None, Some(source)) => self.context.set_document_source(final_url.clone(), source),
                    (None, None) => {
                        log::error!(
                            "Tab[{:?}] navigation produced neither a document nor its source",
                            self.tab_id
                        );
                    }
                }
                self.current_url = Some(final_url.clone());
                // The document's own title, if it has one yet: `self.title` may
                // still be the last page's (a remote page's comes later).
                let visit_title = title.clone().unwrap_or_default();
                if let Some(t) = title.clone() {
                    self.title = t;
                }
                self.is_loading = false;
                self.is_error = false;
                self.state = TabState::Idle;
                self.runtime.dirty = true;

                // Commit to session history. The final URL is used so server redirects
                // collapse into one entry.
                let intent = self
                    .active_nav
                    .as_ref()
                    .filter(|a| a.nav_id == nav_id)
                    .map(|a| a.history)
                    .unwrap_or(HistoryIntent::Push);
                let restore_scroll = match intent {
                    HistoryIntent::Push => {
                        self.history.push(final_url.clone(), title);
                        None
                    }
                    HistoryIntent::Reload => {
                        self.history.replace_current_url(final_url.clone());
                        self.history.set_current_title(title);
                        self.history.current_entry().map(|e| e.scroll)
                    }
                    HistoryIntent::Traverse(entry) => {
                        self.history.replace_current_url(final_url.clone());
                        self.history.set_current_title(title);
                        self.history.entry(entry).map(|e| e.scroll)
                    }
                };
                self.document_entry = self.history.current();
                // Where to land once layout exists: a saved offset wins (returning to an entry
                // the user scrolled), otherwise the URL's fragment, otherwise the top.
                self.pending_scroll = match restore_scroll {
                    Some(offset) if offset != (0, 0) => Some(PendingScroll::Offset(offset.0, offset.1)),
                    _ => final_url
                        .fragment()
                        .filter(|f| !f.is_empty())
                        .map(|f| PendingScroll::Fragment(f.to_string())),
                };

                // Global visited history (URL-bar completion, gosub://history). Only real
                // web pages: internal pages and LoadHtml stand-ins are not "places".
                if let Some(places) = &self.services.places {
                    if matches!(final_url.scheme(), "http" | "https") {
                        places.record_visit(final_url.as_str(), &visit_title);
                    }
                }

                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::Finished { nav_id, url: final_url },
                });
                self.emit_history_changed();
                // set_document cleared hover state; the next mouse move re-derives it.
                self.report_cursor(CursorShape::Default);
                // A new page: whatever the old one's renderer held or focused is gone.
                self.remote_capture = false;
                self.remote_focus = None;
            }
            NavigationResult::Download { nav_id, meta, spooled } => {
                // Not an error and not a page change: the tab stays on its current document
                // and the shell gets a download offer. The Cancelled event stops spinners.
                self.is_loading = false;
                self.is_error = false;
                self.state = TabState::Idle;
                self.pending_url = None;
                self.send_event(EngineEvent::Navigation {
                    tab_id: self.tab_id,
                    event: NavigationEvent::Cancelled {
                        nav_id,
                        url: meta.final_url.clone(),
                        reason: crate::engine::events::CancelReason::Custom("download".into()),
                    },
                });
                let info = crate::net::types::ResponseInfo::from(&meta);
                self.next_offer += 1;
                let pending = PendingDownload {
                    offer: DownloadOfferId(self.next_offer),
                    suggested_filename: info.suggested_filename(),
                    content_type: info.content_type,
                    total_bytes: info.content_length,
                    url: info.final_url,
                };
                self.remember_offer(pending.clone(), spooled);
                self.send_event(EngineEvent::DownloadRequested {
                    tab_id: self.tab_id,
                    offer: pending.offer,
                    url: pending.url,
                    suggested_filename: pending.suggested_filename,
                    content_type: pending.content_type,
                    total_bytes: pending.total_bytes,
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
                        error: error.into(),
                    },
                });
            }
        }
    }

    /// Handle a key press. Keys act on the page (focus traversal, link activation,
    /// scrolling); the shell has already consumed its own shortcuts before forwarding.
    /// Text editing is not here yet - that arrives with the editing slice of M1.
    /// Where a page's link may take the tab, clicked or activated from the
    /// keyboard alike: to the web, or from a file page to another file - never
    /// to an internal page. `None` when it may not, or `href` does not resolve.
    fn page_link_target(&self, href: &str) -> Option<Url> {
        let current = self.current_url.as_ref()?;
        let url = current.join(href).ok()?;
        let allowed =
            matches!(url.scheme(), "http" | "https") || (url.scheme() == "file" && current.scheme() == "file");
        if !allowed {
            log::debug!("link to {href} not followed: scheme not allowed from a page");
            return None;
        }
        Some(url)
    }

    fn handle_key_down(&mut self, key: &str, modifiers: Modifiers) -> ControlFlow {
        // A page a resident renderer retains gets every key its focus might want:
        // all of them while something is focused there, and all but the page-scrolling
        // ones otherwise. Those scroll here, as they would with nothing focused.
        if self.remote_input_available() {
            let scrolls = matches!(
                key,
                "ArrowDown" | "ArrowUp" | "ArrowRight" | "ArrowLeft" | "PageDown" | "PageUp" | " " | "Home" | "End"
            );
            if self.remote_focus.is_some() || !scrolls {
                self.forward_input(InputEvent::KeyDown {
                    key: key.to_string(),
                    modifiers: modifiers.bits(),
                });
                self.runtime.dirty = true;
                return ControlFlow::Continue;
            }
        }
        // The focused control gets non-Tab keys first (typing; more editing follows).
        if key != "Tab" {
            let chord = modifiers.intersects(Modifiers::CONTROL | Modifiers::META);
            let alt = modifiers.contains(Modifiers::ALT);
            let shift = modifiers.contains(Modifiers::SHIFT);
            if self.context.edit_key(key, chord, alt, shift) {
                self.runtime.dirty = true;
                self.runtime.render_now = true;
                self.run_pending_submission();
                self.run_clipboard_traffic();
                self.run_picker_request();
                return ControlFlow::Continue;
            }
            // A clipboard chord can ask for a paste without consuming the key visibly.
            self.run_clipboard_traffic();
        }
        match key {
            // Focus traversal.
            "Tab" => {
                self.context.focus_step(modifiers.contains(Modifiers::SHIFT));
                self.runtime.dirty = true;
                self.runtime.render_now = true;
                self.emit_focus_changed();
                ControlFlow::Continue
            }
            "Escape" => {
                if self.context.set_focus(None, false) {
                    self.runtime.dirty = true;
                    self.runtime.render_now = true;
                    self.emit_focus_changed();
                }
                ControlFlow::Continue
            }
            // Activate a focused link.
            "Enter" => {
                if let Some(url) = self
                    .context
                    .focused_link()
                    .and_then(|href| self.page_link_target(&href))
                {
                    self.navigate_to(url.to_string(), false, HistoryIntent::Push);
                }
                ControlFlow::Continue
            }
            // Page scrolling - only while no text-editable element is focused (an editable
            // will own these keys once editing lands).
            _ if !self.context.focused_editable() => {
                /// One arrow-key step, matching a wheel notch.
                const LINE: f32 = 40.0;
                /// "Almost to the end of the page" - clamped to the real maximum by the
                /// scroll state, so it means "top"/"bottom" for Home/End.
                const FAR: f32 = 1.0e9;
                let page = (self.desired_viewport.height as f32 - LINE).max(LINE);
                let shift = modifiers.contains(Modifiers::SHIFT);
                match key {
                    "ArrowDown" => self.scroll_page_by(0.0, LINE, false),
                    "ArrowUp" => self.scroll_page_by(0.0, -LINE, false),
                    "ArrowRight" => self.scroll_page_by(LINE, 0.0, false),
                    "ArrowLeft" => self.scroll_page_by(-LINE, 0.0, false),
                    "PageDown" => self.scroll_page_by(0.0, page, false),
                    "PageUp" => self.scroll_page_by(0.0, -page, false),
                    " " if shift => self.scroll_page_by(0.0, -page, false),
                    " " => self.scroll_page_by(0.0, page, false),
                    "Home" => self.scroll_page_by(0.0, -FAR, false),
                    "End" => self.scroll_page_by(0.0, FAR, false),
                    _ => ControlFlow::Continue,
                }
            }
            _ => ControlFlow::Continue,
        }
    }

    /// Hold an offer's body until the embedder answers or the tab goes away. Bounded, so an
    /// embedder that ignores offers cannot accumulate temp files without limit; the oldest
    /// offer is evicted first. Dropping a `TempPath` deletes its file.
    fn remember_offer(&mut self, info: PendingDownload, body: tempfile::TempPath) {
        const MAX_PENDING: usize = 8;
        self.pending_offers.push_back(PendingOffer { info, body });
        while self.pending_offers.len() > MAX_PENDING {
            self.pending_offers.pop_front();
        }
        self.publish_pending_downloads();
    }

    fn take_offer(&mut self, offer: DownloadOfferId) -> Option<PendingOffer> {
        let idx = self.pending_offers.iter().position(|p| p.info.offer == offer)?;
        let taken = self.pending_offers.remove(idx);
        self.publish_pending_downloads();
        taken
    }

    /// Mirror the pending offers onto the sink so a shell that missed a
    /// `DownloadRequested` can still find them.
    fn publish_pending_downloads(&self) {
        self.sink
            .set_pending_downloads(self.pending_offers.iter().map(|p| p.info.clone()).collect());
    }

    /// Load a spooled download offer as the page. The override for a misclassified
    /// response; see [`TabCommand::RenderDownload`].
    fn render_download(&mut self, offer: DownloadOfferId) {
        let Some(PendingOffer { info, body }) = self.take_offer(offer) else {
            self.send_event(EngineEvent::Navigation {
                tab_id: self.tab_id,
                event: NavigationEvent::Failed {
                    nav_id: None,
                    url: self.current_url.clone().unwrap_or_else(about_blank),
                    error: LoadError::Content {
                        message: format!("download offer {} is no longer pending", offer.0),
                    },
                },
            });
            return;
        };
        self.begin_local_load(HtmlSource::Spooled(body), info.url);
    }

    /// Save a download to `target_path`: an accepted offer's spooled body (moved into place),
    /// or `url` fetched through the zone fetcher (save-link-as), with progress/finished/failed
    /// events carrying `id`. The fetch runs on its own task; tab shutdown does not cancel it (a
    /// deliberate v1 simplification - there is no cancel command yet).
    ///
    /// V1 fetches in **buffered** mode (whole body in memory before writing): sonar's
    /// `SharedBody` replays nothing to late subscribers, so a streaming consumer that
    /// attaches after the fetch result arrives misses early chunks. True streaming-to-disk
    /// needs replay support in gosub-sonar (see the board).
    fn start_download(
        &mut self,
        id: crate::engine::events::DownloadId,
        url: String,
        target_path: std::path::PathBuf,
        offer: Option<DownloadOfferId>,
    ) {
        let Ok(url) = Url::parse(&url) else {
            self.send_event(EngineEvent::DownloadFailed {
                tab_id: self.tab_id,
                id,
                error: format!("invalid URL: {url}"),
            });
            return;
        };

        if let Some(offer) = offer {
            let Some(PendingOffer { body, .. }) = self.take_offer(offer) else {
                // Never fall back to a fetch: the body the shell accepted is gone.
                self.send_event(EngineEvent::DownloadFailed {
                    tab_id: self.tab_id,
                    id,
                    error: format!("download offer {} is no longer pending", offer.0),
                });
                return;
            };
            // Placement is a rename at best and a full copy across filesystems at worst
            // (temp dir vs. download dir is the common case), so it must not run on the
            // worker: that would stall every command and frame tick for the tab.
            let tab_id = self.tab_id;
            let event_tx = self.zone_context.event_tx.clone();
            spawn_named("tab-download-place", async move {
                let dest = target_path.clone();
                let placed = tokio::task::spawn_blocking(move || place_spooled_download(body, &dest)).await;
                let event = match placed {
                    Ok(Ok(received_bytes)) => EngineEvent::DownloadFinished {
                        tab_id,
                        id,
                        path: target_path,
                        received_bytes,
                    },
                    Ok(Err(e)) => EngineEvent::DownloadFailed {
                        tab_id,
                        id,
                        error: format!("{e:#}"),
                    },
                    Err(e) => EngineEvent::DownloadFailed {
                        tab_id,
                        id,
                        error: format!("placing the download failed: {e}"),
                    },
                };
                let _ = event_tx.send(event);
            });
            return;
        }

        // Save-link-as: no captured body, fetch it now.
        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Other, Initiator::Other);
        // A Download reference routes the transport's per-chunk progress to the shell as
        // DownloadProgress events while the (buffered) fetch is still receiving.
        let reference = RequestReference::Download(id.0);
        self.zone_context
            .request_reference_map
            .write()
            .insert(reference, self.tab_id);
        let req = FetchRequest::builder(Method::GET, url.clone())
            .with_req_id(req_id)
            .with_reference(REF_REGISTRY.to_net(reference))
            .with_priority(Priority::Low)
            .with_kind(ResourceKind::Other.to_net())
            .with_initiator(Initiator::Other.to_net())
            .with_streaming(false)
            .with_auto_decode(true)
            .build();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let reference_map = self.zone_context.request_reference_map.clone();
        spawn_named("tab-download", async move {
            // Remove the routing entry however the download ends.
            let _cleanup = scopeguard(move || {
                reference_map.write().remove(&reference);
            });

            let fail = |error: String| {
                let _ = event_tx.send(EngineEvent::DownloadFailed { tab_id, id, error });
            };

            let result = match submit_to_io(zone_id, Some(tab_id), req, io_tx, None).await {
                Ok((_handle, rx)) => match rx.await {
                    Ok(result) => result,
                    Err(_) => return fail("fetch channel closed".into()),
                },
                Err(e) => return fail(format!("submit failed: {e}")),
            };

            match result {
                FetchResult::Stream { meta, peek_buf, shared } => {
                    if meta.status != 200 {
                        return fail(format!("HTTP {} {}", meta.status, meta.status_text));
                    }
                    if let Err(e) = stream_to_file(
                        id,
                        tab_id,
                        &event_tx,
                        meta.content_length,
                        peek_buf,
                        shared,
                        &target_path,
                    )
                    .await
                    {
                        fail(e.to_string());
                    }
                }
                FetchResult::Buffered { meta, body } => {
                    if meta.status != 200 {
                        return fail(format!("HTTP {} {}", meta.status, meta.status_text));
                    }
                    // Buffered mode: the body is complete, so progress is a single report
                    // (keeps the shell's event sequence uniform with a streaming future).
                    let _ = event_tx.send(EngineEvent::DownloadProgress {
                        tab_id,
                        id,
                        received_bytes: body.len() as u64,
                        total_bytes: Some(body.len() as u64),
                    });
                    if let Err(e) = tokio::fs::write(&target_path, &body).await {
                        return fail(format!("write {}: {e}", target_path.display()));
                    }
                    let _ = event_tx.send(EngineEvent::DownloadFinished {
                        tab_id,
                        id,
                        path: target_path,
                        received_bytes: body.len() as u64,
                    });
                }
                FetchResult::Error(e) => fail(e.to_string()),
            }
        });
    }

    /// Tell the shell where keyboard focus went (e.g. to drive IME/on-screen keyboards).
    fn emit_focus_changed(&self) {
        self.send_event(EngineEvent::FocusChanged {
            tab_id: self.tab_id,
            focused: self.context.focused_node().is_some(),
            editable: self.context.focused_editable(),
        });
    }

    /// Scroll the page by a CSS-px delta - shared by wheel scrolling and keyboard
    /// scrolling. Uses the zero-copy TileCache fast path when only the offset changed.
    fn scroll_page_by(&mut self, delta_x: f32, delta_y: f32, precise: bool) -> ControlFlow {
        // When page height is known, clamp to the real maximum so worker and context
        // stay in sync. When the page hasn't rendered yet, allow free scrolling (the
        // context will clamp to the actual page height on its own).
        let max_y = {
            let ph = self.context.page_height();
            if ph > 0.0 {
                (ph - self.desired_viewport.height as f64).max(0.0)
            } else {
                f64::MAX
            }
        };
        // The same for sideways: a page no wider than the viewport does not scroll sideways at
        // all. Unclamped, a touch swipe that drifts sideways slid the page off into blank space.
        let max_x = {
            let pw = self.context.page_width();
            if pw > 0.0 {
                (pw - self.desired_viewport.width as f64).max(0.0)
            } else {
                f64::MAX
            }
        };

        match self
            .scroll
            .scroll_by(delta_x as f64, delta_y as f64, max_x, max_y, precise)
        {
            // Instant behavior: apply the new offset now and keep the immediate-submit fast
            // path (avoids up to 1/fps of latency per scroll event).
            Some((x, y)) => {
                // A precise delta may have cut a wheel animation short; the next one must not
                // start from that animation's stale timestamp.
                self.scroll_anim_last = None;
                let moved = x != self.scroll_x || y != self.scroll_y;
                self.scroll_x = x;
                self.scroll_y = y;
                self.context.set_scroll(x as f64, y as f64);

                // GPU-tile-compositing backends skip this CPU TileCache fast path (their
                // tiles have no CPU pixels); they re-composite on the next tick.
                if self.zone_context.render_backend.raster_strategy() != RasterStrategy::None
                    && !self.zone_context.render_backend.gpu_tile_compositing()
                {
                    let dpr = self.zone_context.render_backend.device_pixel_ratio();
                    // Same DPR guard as the render path: cached tiles rasterized at a different
                    // DPR are the wrong physical size for this frame.
                    self.context.invalidate_raster_if_dpr_changed(dpr);
                    if let Some(handle) = self.context.take_scroll_handle(dpr) {
                        self.runtime.committed_scene_epoch = self.context.scene_epoch();
                        self.submit_frame(handle);
                        return ControlFlow::Continue;
                    }
                }

                // TileCache not ready yet; fall back to the timer path. Only mark dirty if
                // the integer offset actually moved (sub-pixel deltas are no-ops).
                if moved {
                    self.runtime.dirty = true;
                }
            }
            // Animated behavior: tick_draw advances the ease toward the new target. Request
            // an immediate tick so the first frame lands without waiting up to 1/fps.
            None => {
                self.runtime.render_now = true;
            }
        }
        ControlFlow::Continue
    }

    fn handle_tab_command(&mut self, cmd: TabCommand) -> ControlFlow {
        match cmd {
            TabCommand::CloseTab => ControlFlow::Break,
            TabCommand::SetTitle { title } => {
                self.title = title;
                self.publish_tab_state();
                ControlFlow::Continue
            }
            TabCommand::Navigate { url } => {
                self.navigate_to(&url, false, HistoryIntent::Push);
                ControlFlow::Continue
            }
            TabCommand::LoadHtml { html, base_url } => {
                self.load_html(html, base_url);
                ControlFlow::Continue
            }
            TabCommand::Reload { ignore_cache } => {
                let url = self
                    .current_url
                    .as_ref()
                    .map(|u| u.as_str())
                    .unwrap_or("about:blank")
                    .to_string();
                self.navigate_to(url.as_str(), ignore_cache, HistoryIntent::Reload);
                ControlFlow::Continue
            }
            TabCommand::GoBack => {
                self.history.set_current_scroll(self.scroll_x, self.scroll_y);
                if let Some(entry) = self.history.go_back() {
                    self.traverse_history(entry);
                }
                ControlFlow::Continue
            }
            TabCommand::GoForward { entry } => {
                self.history.set_current_scroll(self.scroll_x, self.scroll_y);
                if let Some(entry) = self.history.go_forward(entry) {
                    self.traverse_history(entry);
                }
                ControlFlow::Continue
            }
            TabCommand::GoToHistoryEntry { entry } => {
                if Some(entry) != self.history.current() {
                    self.history.set_current_scroll(self.scroll_x, self.scroll_y);
                    if let Some(entry) = self.history.go_to(entry) {
                        self.traverse_history(entry);
                    }
                }
                ControlFlow::Continue
            }
            TabCommand::StartDownload {
                id,
                url,
                target_path,
                offer,
            } => {
                self.start_download(id, url, target_path, offer);
                ControlFlow::Continue
            }
            #[cfg(test)]
            TabCommand::CrashForTest => panic!("deliberate test crash"),
            TabCommand::QueryHitTest { x, y, token } => {
                let hit = self.context.hit_test(x as f64, y as f64, self.current_url.as_ref());
                self.send_event(EngineEvent::HitTestResult {
                    tab_id: self.tab_id,
                    token,
                    hit,
                });
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
            TabCommand::MouseScroll {
                delta_x,
                delta_y,
                precise,
            } => {
                // A resident renderer holding the pointer (an open dropdown) takes the wheel.
                if self.remote_capture {
                    if let Some((px, py)) = self.context.pointer() {
                        if self.forward_input(InputEvent::Wheel {
                            x: px,
                            y: py,
                            delta_y: delta_y as f64,
                        }) {
                            self.runtime.dirty = true;
                            return ControlFlow::Continue;
                        }
                    }
                }
                // An open dropdown, or a scrolling textarea, under the pointer takes the wheel.
                if let Some((px, py)) = self.context.pointer() {
                    if self.context.popup_scroll(px, py, delta_y as f64)
                        || self.context.area_scroll(px, py, delta_y as f64)
                    {
                        self.runtime.dirty = true;
                        self.runtime.render_now = true;
                        return ControlFlow::Continue;
                    }
                }
                self.scroll_page_by(delta_x, delta_y, precise)
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
                // A resident renderer holding the pointer gets the move; the page's own
                // gestures and popups live there, and it answers with the cursor.
                if self.remote_capture
                    && self.forward_input(InputEvent::PointerMove {
                        x: x as f64,
                        y: y as f64,
                    })
                {
                    self.runtime.dirty = true;
                    return ControlFlow::Continue;
                }
                // Each of these must run: a drag doesn't get to skip a move because hover changed.
                let popup_dirty = self.context.popup_hover_at(x as f64, y as f64);
                let drag_dirty = self.context.drag_move(x as f64, y as f64);
                let cursor = self.context.cursor_at(x as f64, y as f64);
                self.report_cursor(cursor);
                if visual_dirty || popup_dirty || drag_dirty {
                    self.runtime.dirty = true;
                    // A resize re-layouts the page; let the frame tick pace it so a burst of
                    // pointer events collapses into one render instead of one each.
                    self.runtime.render_now = !self.context.is_resizing();
                }
                ControlFlow::Continue
            }
            TabCommand::MouseDown { x, y, button } => {
                if matches!(button, crate::events::MouseButton::Left) {
                    // A page a resident renderer retains takes the press there, every
                    // press: focus and blur, a link under the pointer, activation, and
                    // what they ask for come back as effects the broker judges. Nothing
                    // here has a document to focus, and a link followed from here would
                    // leave the renderer's focus where it was.
                    if self.forward_input(InputEvent::PointerDown {
                        x: x as f64,
                        y: y as f64,
                        button,
                    }) {
                        self.runtime.dirty = true;
                        return ControlFlow::Continue;
                    }
                    // Click-to-focus (or blur when the click lands on nothing focusable),
                    // before any link activation.
                    let focused = self.context.focus_at(x as f64, y as f64);
                    if focused {
                        self.emit_focus_changed();
                    }
                    if let Some(href) = self.context.hover_link_url.clone() {
                        if let Some(url) = self.page_link_target(&href) {
                            self.navigate_to(url.to_string(), false, HistoryIntent::Push);
                        }
                        return ControlFlow::Continue;
                    }
                    // Activation (checkbox/radio toggles) lands in the same render as the focus.
                    let toggled = self.context.activate_at(x as f64, y as f64);
                    if focused || toggled {
                        self.runtime.render_now = true;
                    }
                    self.run_pending_submission();
                    self.run_picker_request();
                }
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::KeyDown { key, modifiers, .. } => self.handle_key_down(&key, modifiers),
            TabCommand::TextInput { text } => {
                if self.forward_input(InputEvent::Text { text: text.clone() }) {
                    self.runtime.dirty = true;
                    return ControlFlow::Continue;
                }
                if self.context.insert_text(&text) {
                    self.runtime.render_now = true;
                }
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::PickerChanged { value } => {
                if self.forward_input(InputEvent::PickerChanged { value: value.clone() }) {
                    self.runtime.dirty = true;
                    return ControlFlow::Continue;
                }
                if self.context.set_picker_value(&value) {
                    self.runtime.dirty = true;
                    self.runtime.render_now = true;
                }
                ControlFlow::Continue
            }
            TabCommand::PickerClosed => {
                if !self.forward_input(InputEvent::PickerClosed) {
                    self.context.end_picker();
                }
                ControlFlow::Continue
            }
            TabCommand::SetScroll { x, y } => {
                self.apply_scroll(x, y);
                ControlFlow::Continue
            }
            TabCommand::MouseUp { x, y, button } => {
                // Ends a gesture on either side; a resident renderer answers with the
                // capture released.
                self.forward_input(InputEvent::PointerUp {
                    x: x as f64,
                    y: y as f64,
                    button,
                });
                self.context.end_drag();
                self.runtime.dirty = true;
                ControlFlow::Continue
            }
            TabCommand::KeyUp { .. } => {
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
                    log::warn!("Cancelling in-flight load for tab {:?}", self.tab_id);
                    load.cancel.cancel();
                }
                ControlFlow::Continue
            }
            TabCommand::RenderDownload { offer } => {
                self.render_download(offer);
                ControlFlow::Continue
            }
            #[cfg(feature = "unstable-api")]
            _ => {
                log::warn!("Tab {:?} received unhandled command: {:?}", self.tab_id, cmd);
                ControlFlow::Continue
            }
        }
    }

    /// Hand a finished frame to the zone's compositor sink and ring the wakeup.
    ///
    /// The sink is the data channel (it holds the pixels); [`EngineEvent::Redraw`] is only
    /// the wakeup. Shells present by asking the sink for the tab's current frame.
    fn submit_frame(&self, handle: ExternalHandle) {
        self.zone_context.compositor.submit_frame(self.tab_id, handle);
        self.send_event(EngineEvent::Redraw { tab_id: self.tab_id });
    }

    /// Republish the tab's externally-readable state (URL, title, back/forward availability)
    /// so [`TabHandle`](crate::tab::TabHandle) accessors answer without replaying events.
    fn publish_tab_state(&self) {
        self.sink.set_url(self.current_url.clone());
        self.sink.set_title(self.title.clone());
        self.sink
            .set_history_flags(self.history.can_go_back(), self.history.can_go_forward());
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

    /// Load the history entry the cursor was just moved to (back/forward/jump). The entry's URL
    /// is refetched; its saved scroll offset is restored once the load commits.
    fn traverse_history(&mut self, entry: HistoryEntryId) {
        let Some(url) = self.history.entry(entry).map(|e| e.url.to_string()) else {
            return;
        };
        self.navigate_to(url, false, HistoryIntent::Traverse(entry));
        // The cursor moved even though the load is still in flight: tell the shell now so
        // back/forward buttons track the traversal, not the eventual load.
        self.emit_history_changed();
    }

    /// Forward Ctrl+C/X text to the embedder and relay a Ctrl+V as a paste request; the
    /// embedder answers the latter with `TabCommand::TextInput`.
    fn run_clipboard_traffic(&mut self) {
        if let Some(text) = self.context.take_clipboard_write() {
            self.send_event(EngineEvent::ClipboardWrite {
                tab_id: self.tab_id,
                text,
            });
        }
        if self.context.take_paste_request() {
            self.send_event(EngineEvent::PasteRequested { tab_id: self.tab_id });
        }
    }

    /// Tell the embedder an input wants its picker, if the last click/key activated one.
    fn run_picker_request(&mut self) {
        if let Some(req) = self.context.take_picker_request() {
            self.send_event(EngineEvent::PickerRequested {
                tab_id: self.tab_id,
                kind: req.kind,
                x: req.anchor.x as f32,
                y: req.anchor.y as f32,
                width: req.anchor.width as f32,
                height: req.anchor.height as f32,
                value: req.value,
                min: req.min,
                max: req.max,
                step: req.step,
            });
        }
    }

    /// Run a form submission the browsing context queued for the last click/key. Pushes a
    /// history entry like any fresh navigation.
    /// Whether this tab's input goes to a resident renderer's retained page.
    fn remote_input_available(&self) -> bool {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            self.context.remote_input_available()
        }
        #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
        {
            false
        }
    }

    /// Send one input event to the resident renderer retaining this tab's page.
    /// False when there is none, so the caller handles the input in-process.
    fn forward_input(&mut self, event: InputEvent) -> bool {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            self.context.remote_input(event)
        }
        #[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
        {
            let _ = event;
            false
        }
    }

    /// Act on what input passes asked for, each judged first (see
    /// [`remote_effects`](crate::engine::tab::remote_effects)).
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn apply_remote_effects(&mut self) {
        use crate::engine::tab::remote_effects::{action_for, Action};
        let viewport = (self.desired_viewport.width as f64, self.desired_viewport.height as f64);
        for (provenance, effect) in self.context.take_remote_effects() {
            match action_for(effect, provenance, self.current_url.as_ref(), viewport) {
                Ok(Action::Focus { focused, editable }) => {
                    self.remote_focus = focused.then_some(editable);
                    self.send_event(EngineEvent::FocusChanged {
                        tab_id: self.tab_id,
                        focused,
                        editable,
                    });
                }
                Ok(Action::Cursor(cursor)) => self.report_cursor(cursor),
                Ok(Action::Navigate { url, method, body }) => {
                    self.navigate_request(
                        url.to_string(),
                        method,
                        body.map(RequestBody::form),
                        HistoryIntent::Push,
                    );
                }
                Ok(Action::Picker {
                    kind,
                    bounds,
                    value,
                    min,
                    max,
                    step,
                }) => self.send_event(EngineEvent::PickerRequested {
                    tab_id: self.tab_id,
                    kind,
                    x: bounds.x as f32,
                    y: bounds.y as f32,
                    width: bounds.width as f32,
                    height: bounds.height as f32,
                    value,
                    min,
                    max,
                    step,
                }),
                Ok(Action::ClipboardWrite(text)) => self.send_event(EngineEvent::ClipboardWrite {
                    tab_id: self.tab_id,
                    text,
                }),
                Ok(Action::PasteRequested) => self.send_event(EngineEvent::PasteRequested { tab_id: self.tab_id }),
                Ok(Action::Capture(pointer)) => self.remote_capture = pointer,
                Err(why) => log::warn!("the renderer asked for {why}; ignored"),
            }
        }
    }

    fn run_pending_submission(&mut self) {
        let Some(sub) = self.context.take_submission() else {
            return;
        };
        let (method, body) = if sub.post {
            (Method::POST, sub.body.map(RequestBody::form))
        } else {
            (Method::GET, None)
        };
        self.navigate_request(sub.url.to_string(), method, body, HistoryIntent::Push);
    }

    /// Emit `CursorChanged` if the shape differs from the last one reported.
    fn report_cursor(&mut self, cursor: CursorShape) {
        if cursor != self.reported_cursor {
            self.reported_cursor = cursor;
            self.send_event(EngineEvent::CursorChanged {
                tab_id: self.tab_id,
                cursor,
            });
        }
    }

    /// Broadcast the current history snapshot to the embedder.
    fn emit_history_changed(&self) {
        self.publish_tab_state();
        self.send_event(EngineEvent::Navigation {
            tab_id: self.tab_id,
            event: NavigationEvent::HistoryChanged {
                history: self.history.snapshot(),
            },
        });
    }

    /// Whether `url` differs from the loaded document's URL only in its fragment - a
    /// "navigate to a fragment" per HTML, which must not refetch the document.
    fn is_same_document(&self, url: &Url) -> bool {
        match (&self.current_url, self.is_loading) {
            (Some(cur), false) => {
                let mut a = cur.clone();
                let mut b = url.clone();
                a.set_fragment(None);
                b.set_fragment(None);
                a == b
            }
            _ => false,
        }
    }

    /// Same-document (fragment) navigation: no fetch. Updates the current URL, records
    /// history like a real navigation would (fresh navigations push, traversals just moved
    /// the cursor), scrolls to the indicated part, and reports the navigation as finished so
    /// the shell updates its address bar.
    fn navigate_same_document(&mut self, url: Url, history: HistoryIntent) {
        let nav_id = NavigationId::new();
        self.send_event(EngineEvent::Navigation {
            tab_id: self.tab_id,
            event: NavigationEvent::Started {
                nav_id,
                url: url.clone(),
            },
        });

        match history {
            HistoryIntent::Push => {
                self.history.set_current_scroll(self.scroll_x, self.scroll_y);
                self.history.push(url.clone(), Some(self.title.clone()));
            }
            HistoryIntent::Reload | HistoryIntent::Traverse(_) => {}
        }
        self.current_url = Some(url.clone());

        // Layout already exists, so the target can be scrolled to right away. A traversal to
        // an entry restores its saved offset instead (the user may have scrolled after
        // arriving at the fragment).
        let target = match history {
            HistoryIntent::Traverse(entry) => self.history.entry(entry).map(|e| e.scroll),
            _ => url
                .fragment()
                .and_then(|f| self.context.fragment_target_y(f))
                .map(|y| (0, y.round() as i32)),
        };
        if let Some((x, y)) = target {
            self.apply_scroll(x, y);
        }

        self.send_event(EngineEvent::Navigation {
            tab_id: self.tab_id,
            event: NavigationEvent::Finished { nav_id, url },
        });
        self.emit_history_changed();
    }

    /// Set the scroll offset immediately (clamped by the context) and re-render.
    fn apply_scroll(&mut self, x: i32, y: i32) {
        self.context.set_scroll(x as f64, y as f64);
        let (cx, cy) = self.context.scroll_xy();
        self.scroll_x = cx.round() as i32;
        self.scroll_y = cy.round() as i32;
        self.scroll.reset(cx, cy);
        self.scroll_anim_last = None;
        self.runtime.dirty = true;
        self.runtime.render_now = true;
    }

    /// Navigate to a new URL, cancelling any in-flight navigation. `history` says what the
    /// navigation does to session history once it commits.
    fn navigate_to(&mut self, url: impl Into<String>, ignore_cache: bool, history: HistoryIntent) {
        let _ = ignore_cache;
        self.navigate_request(url, Method::GET, None, history);
    }

    /// Navigate with an explicit method and optional body (form POSTs), cancelling any
    /// in-flight navigation.
    fn navigate_request(
        &mut self,
        url: impl Into<String>,
        method: Method,
        body: Option<RequestBody>,
        history: HistoryIntent,
    ) {
        let url = match self.parse_url(url.into()) {
            Ok(u) => u,
            Err(_) => return,
        };

        // A fragment-only change of the loaded document does not refetch it. Reloads always
        // refetch (that is what reload means).
        if history != HistoryIntent::Reload && self.is_same_document(&url) {
            self.navigate_same_document(url, history);
            return;
        }

        // Leaving the current entry: remember where the user was so back/forward can restore
        // it. (Traversals already saved it before moving the cursor - see `traverse_history`.)
        if !matches!(history, HistoryIntent::Traverse(_)) {
            self.history.set_current_scroll(self.scroll_x, self.scroll_y);
        }
        self.pending_scroll = None;
        self.reset_scroll_for_navigation();
        // Cancel any previous running navigation in this tab
        self.cancel_current_nav();

        // gosub:// and about: pages are served by the engine's page registry, never fetched.
        if InternalPages::handles(&url) {
            let (tile_count, tile_bytes) = self.context.tile_stats();
            let tab_view = TabView {
                history: self.history.snapshot(),
                render_backend: self.zone_context.render_backend.name(),
                stats: crate::engine::internal_pages::TabStats {
                    viewport_width: self.desired_viewport.width,
                    viewport_height: self.desired_viewport.height,
                    scroll_x: self.scroll_x as f64,
                    scroll_y: self.scroll_y as f64,
                    page_height: self.context.page_height(),
                    tile_count,
                    tile_bytes,
                    scene_epoch: self.context.scene_epoch(),
                    raster_dpr: self.zone_context.render_backend.device_pixel_ratio(),
                },
            };
            let page = self
                .zone_context
                .internal_pages
                .resolve(&url, &self.zone_context.config_store, &tab_view);
            self.load_html_document(page.html, url, history);
            return;
        }

        if let Err(e) = self.bind_storage_for(url.clone()) {
            self.send_event(EngineEvent::Navigation {
                tab_id: self.tab_id,
                event: NavigationEvent::Failed {
                    nav_id: None,
                    url: url.clone(),
                    error: LoadError::Io {
                        message: format!("{e:#}"),
                    },
                },
            });
            return;
        }

        let nav_id = NavigationId::new();

        // Start this navigation's clock. Every page.* mark is measured from here, so it
        // must be stamped before any work, not when the document arrives.
        let scope = gosub_shared::timing::ScopeId(nav_id.0);
        gosub_shared::timing::begin_scope(scope);
        self.timing_scope = Some(scope);
        self.first_paint_marked = false;

        let parent_cancel = CancellationToken::new();
        self.active_nav = Some(ActiveNav {
            nav_id,
            cancel: parent_cancel.clone(),
            url: url.clone(),
            history,
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

        // This tab is now loading `url`, so requests it makes are attributed to
        // that document. Announced before submitting, so the navigation request
        // itself is already attributed. Cookies are attached I/O-side from here on -
        // see `net::tab_identity`.
        self.announce_top_level(&url);

        let mut fetch_headers = HeaderMap::new();
        if let Some(langs) = &self.services.accept_language {
            if let Ok(val) = langs.parse() {
                fetch_headers.insert(http::header::ACCEPT_LANGUAGE, val);
            }
        }
        if let Ok(val) = ResourceKind::Document.accept_header().parse() {
            fetch_headers.insert(http::header::ACCEPT, val);
        }

        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Document, Initiator::Navigation);
        let mut req = FetchRequest::builder(method, url.clone())
            .with_reference(REF_REGISTRY.to_net(RequestReference::Navigation(nav_id)))
            .with_req_id(req_id)
            .with_headers(fetch_headers)
            .with_priority(Priority::High)
            .with_kind(ResourceKind::Document.to_net())
            .with_initiator(Initiator::Navigation.to_net())
            // Use buffered mode so the full document body is available before parsing.
            // The streaming path has a race where SharedBody can close before parse_stream
            // subscribes, causing truncated HTML (only the 5 KB peek buffer is parsed).
            .with_streaming(false)
            .with_auto_decode(true);
        if let Some(body) = body {
            req = req.with_body(body);
        }
        let req = req.build();

        let (tx_done, rx_done) = oneshot::channel::<NavigationResult<C>>();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let accept_language = self.services.accept_language.clone();
        let max_document_bytes = self.zone_context.config_store.get_uint("net.document.max_bytes");
        let max_download_spool_bytes = self.zone_context.config_store.get_uint("net.download.max_spool_bytes") as u64;
        let font_system = self.zone_context.font_system.clone();
        // Capture the document source only when a renderer process may need it
        // (it re-parses there); otherwise skip the copy.
        let capture_source = self.remote_render_available();

        let span = tracing::info_span!(
            "tab_nav",
            tab_id=%tab_id,
            nav_id=%nav_id.0,
            scheme=%url.scheme(),
            host=%url.host_str().unwrap_or(""),
            path=%url.path(),
        );

        let parent_cancel_clone = parent_cancel.clone();

        // Spawn the actual fetcher into a separate task
        spawn_named("tab-fetcher", async move {
            let _enter = span.enter();

            let submit = submit_to_io(
                zone_id,
                Some(tab_id),
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
                allow_download_without_user_activation: false,
            };

            let mut hooks = ResourcePipelines::<C>::new(
                zone_id,
                tab_id,
                io_tx.clone(),
                accept_language.clone(),
                max_document_bytes,
                max_download_spool_bytes,
                font_system.clone(),
                capture_source,
                capture_source,
            );

            // The URL a source-only document lands on: the response's, after redirects.
            let document_final_url = fetch_result
                .meta()
                .map(|meta| meta.final_url.clone())
                .unwrap_or_else(about_blank);
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
                Ok(RoutedOutcome::MainDocument { doc, source }) => {
                    use gosub_interface::document::Document as _;
                    let final_url = doc
                        .as_ref()
                        .and_then(|doc| doc.url())
                        .unwrap_or_else(|| document_final_url.clone());
                    let title = doc.as_ref().and_then(|doc| crate::html::document_title(doc));
                    let _ = tx_done.send(NavigationResult::Ok {
                        nav_id,
                        final_url,
                        title,
                        doc,
                        source,
                    });
                }
                Ok(RoutedOutcome::DownloadOffer { meta, spooled }) => {
                    let _ = tx_done.send(NavigationResult::Download {
                        nav_id,
                        meta: *meta,
                        spooled,
                    });
                }
                Ok(RoutedOutcome::ViewerRendered(_doc)) => {
                    log::warn!("Tab[{:?}] viewer rendering not supported yet", tab_id);
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Viewer rendering not supported yet")),
                    });
                }
                // Subresource outcomes need no main-frame navigation handling.
                Ok(RoutedOutcome::CssLoaded(_) | RoutedOutcome::ScriptExecuted(_) | RoutedOutcome::FontLoaded(_)) => {
                    log::trace!("Tab[{:?}] subresource outcome; nothing to do for navigation", tab_id);
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
                            error: LoadError::Blocked { reason },
                        },
                    });
                }
                Err(e) => {
                    // The router wraps a `NetError` in `anyhow` on the fetch-failure path;
                    // recover it so the classification (blocked, timeout, cancelled, I/O)
                    // survives instead of becoming a message string.
                    let error = match e.downcast_ref::<NetError>() {
                        Some(net) => NavigationError::Net(net.clone()),
                        None => NavigationError::NetworkError(format!("Routing error: {e}")),
                    };
                    let _ = tx_done.send(NavigationResult::Err { nav_id, error });
                }
            }
        });

        self.load = Some(NavJoin {
            cancel: parent_cancel.clone(),
            rx: Some(rx_done),
        });
    }

    /// Load caller-supplied HTML into the tab, bypassing the network. The document is
    /// parsed through the regular HTML pipeline (so subresources like stylesheets and
    /// images are still discovered and fetched, resolved against `base_url`) and
    /// completes through the same navigation path as `navigate_to`.
    /// `TabCommand::LoadHtml`: caller-supplied HTML as a fresh navigation to `base_url`.
    fn load_html(&mut self, html: String, base_url: String) {
        let url = match self.parse_url(base_url) {
            Ok(u) => u,
            Err(_) => return,
        };
        self.begin_local_load(HtmlSource::Text(html), url);
    }

    /// A fresh navigation to locally supplied HTML (LoadHtml, or a rendered download offer).
    fn begin_local_load(&mut self, source: HtmlSource, url: Url) {
        self.history.set_current_scroll(self.scroll_x, self.scroll_y);
        self.pending_scroll = None;
        self.reset_scroll_for_navigation();
        self.cancel_current_nav();
        self.load_document_source(source, url, HistoryIntent::Push);
    }

    /// Reset scroll state at the start of a navigation.
    fn reset_scroll_for_navigation(&mut self) {
        self.scroll_x = 0;
        self.scroll_y = 0;
        self.scroll.reset(0.0, 0.0);
        self.scroll_anim_last = None;
        self.context.reset_scroll();
    }

    /// Parse `html` as the document for `url` without touching the network, with `history`
    /// deciding what the commit does to session history. Shared by `LoadHtml` (always a push)
    /// and `gosub://` internal pages (push, reload or traversal like any navigation). The
    /// caller has already reset scroll and cancelled the previous navigation.
    /// Tell the I/O side which document this tab's requests now belong to. Queued
    /// behind the fetches already submitted, so those keep the document they were
    /// made for; set directly only when the I/O side is gone and nothing is queued.
    fn announce_top_level(&self, url: &Url) {
        let announced = self.zone_context.io_tx.send(IoCommand::SetTopLevel {
            tab_id: self.tab_id,
            url: url.clone(),
        });
        if announced.is_err() {
            self.zone_context.tab_identities.set_top_level(self.tab_id, url.clone());
        }
    }

    fn load_html_document(&mut self, html: String, url: Url, history: HistoryIntent) {
        self.load_document_source(HtmlSource::Text(html), url, history);
    }

    fn load_document_source(&mut self, source: HtmlSource, url: Url, history: HistoryIntent) {
        if let Err(e) = self.bind_storage_for(url.clone()) {
            self.send_event(EngineEvent::Navigation {
                tab_id: self.tab_id,
                event: NavigationEvent::Failed {
                    nav_id: None,
                    url: url.clone(),
                    error: LoadError::Io {
                        message: format!("{e:#}"),
                    },
                },
            });
            return;
        }

        let nav_id = NavigationId::new();

        // Start this navigation's clock. Every page.* mark is measured from here, so it
        // must be stamped before any work, not when the document arrives.
        let scope = gosub_shared::timing::ScopeId(nav_id.0);
        gosub_shared::timing::begin_scope(scope);
        self.timing_scope = Some(scope);
        self.first_paint_marked = false;

        let parent_cancel = CancellationToken::new();
        // Its subresources are this document's, not whatever the tab showed before:
        // the same announcement a network navigation makes.
        self.announce_top_level(&url);

        self.active_nav = Some(ActiveNav {
            nav_id,
            cancel: parent_cancel.clone(),
            url: url.clone(),
            history,
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

        // Synthetic request/response pair so the HTML pipeline can attribute the parse
        // and its discovered subresources to this navigation.
        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Document, Initiator::Navigation);
        let req = FetchRequest::builder(Method::GET, url.clone())
            .with_reference(REF_REGISTRY.to_net(RequestReference::Navigation(nav_id)))
            .with_req_id(req_id)
            .with_priority(Priority::High)
            .with_kind(ResourceKind::Document.to_net())
            .with_initiator(Initiator::Navigation.to_net())
            .with_streaming(false)
            .with_auto_decode(false)
            .build();

        let (tx_done, rx_done) = oneshot::channel::<NavigationResult<C>>();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let accept_language = self.services.accept_language.clone();
        let max_document_bytes = self.zone_context.config_store.get_uint("net.document.max_bytes");
        let max_download_spool_bytes = self.zone_context.config_store.get_uint("net.download.max_spool_bytes") as u64;
        let font_system = self.zone_context.font_system.clone();
        // Same rule as navigate(): keep the source only when a renderer process may re-parse it.
        let capture_source = self.remote_render_available();
        // The engine's own pages are parsed here too: they are the ones allowed to
        // fall back to in-process rendering, which needs a document. `LoadHtml`
        // content is the embedder's page and goes to the renderer like any other.
        let source_only = capture_source && !matches!(url.scheme(), "gosub" | "about");

        let span = tracing::info_span!(
            "tab_load_html",
            tab_id=%tab_id,
            nav_id=%nav_id.0,
            base_url=%url,
        );

        // The pipeline cancels the handle token after parsing to reap subresource
        // children, so give it a child of the navigation token: CancelNavigation still
        // aborts the parse, but the pipeline's post-parse cancel doesn't kill the
        // navigation token itself.
        let handle = FetchHandle {
            req_id,
            cancel: parent_cancel.child_token(),
        };
        spawn_named("tab-load-html", async move {
            let _enter = span.enter();

            // A spooled body is read here, off the worker, and capped like a fetched document.
            let html = match source {
                HtmlSource::Text(html) => html,
                HtmlSource::Spooled(path) => {
                    use tokio::io::AsyncReadExt;
                    let read = async {
                        let file = tokio::fs::File::open(&path).await?;
                        let mut bytes = Vec::new();
                        file.take(max_document_bytes as u64).read_to_end(&mut bytes).await?;
                        Ok::<_, std::io::Error>(bytes)
                    };
                    match read.await {
                        // Unknown encoding; the parser's own sniffing would be better.
                        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                        Err(e) => {
                            let _ = tx_done.send(NavigationResult::Err {
                                nav_id,
                                error: NavigationError::Io(e),
                            });
                            return;
                        }
                    }
                }
            };
            let meta = {
                let mut meta = FetchResultMeta::synthetic(url.clone());
                meta.content_length = Some(html.len() as u64);
                meta.content_type = Some("text/html".into());
                meta.has_body = true;
                meta
            };

            let mut hooks = ResourcePipelines::<C>::new(
                zone_id,
                tab_id,
                io_tx.clone(),
                accept_language.clone(),
                max_document_bytes,
                max_download_spool_bytes,
                font_system.clone(),
                capture_source,
                source_only,
            );

            match hooks.html.parse_bytes(req, handle, meta, html.as_bytes()).await {
                Ok(parsed) => {
                    use gosub_interface::document::Document as _;
                    let (doc, source) = parsed.into_parts();
                    let doc = doc.map(Arc::new);
                    let final_url = doc.as_ref().and_then(|doc| doc.url()).unwrap_or(url);
                    let title = doc.as_ref().and_then(|doc| crate::html::document_title(doc));
                    let _ = tx_done.send(NavigationResult::Ok {
                        nav_id,
                        final_url,
                        title,
                        doc,
                        source,
                    });
                }
                Err(e) => {
                    let _ = tx_done.send(NavigationResult::Err {
                        nav_id,
                        error: NavigationError::Other(anyhow!("Failed to parse HTML: {e}")),
                    });
                }
            }
        });

        self.load = Some(NavJoin {
            cancel: parent_cancel.clone(),
            rx: Some(rx_done),
        });
    }

    /// Title and icon of a document the renderer process parsed, once its
    /// first render reports them.
    #[cfg(all(feature = "process-isolation", target_os = "linux"))]
    fn apply_remote_document_meta(&mut self) {
        let Some((title, favicon)) = self.context.take_remote_document_meta() else {
            return;
        };
        if let Some(title) = title {
            // The history entry was committed before anything was parsed, so it
            // learns its title here - also when the tab's title does not change
            // (the last page may have had the same one) - and the embedder gets
            // the snapshot that has it. The document's entry, not the cursor's:
            // a traversal moves the cursor before its own load commits.
            let untitled = self.document_entry.filter(|id| {
                self.history
                    .entry(*id)
                    .is_some_and(|e| e.title.as_deref() != Some(title.as_str()))
            });
            if let Some(id) = untitled {
                self.history.set_entry_title(id, Some(title.clone()));
                // The visit was counted at the commit; only its title is new.
                if let (Some(places), Some(url)) = (&self.services.places, &self.current_url) {
                    if matches!(url.scheme(), "http" | "https") {
                        places.set_visit_title(url.as_str(), &title);
                    }
                }
                self.emit_history_changed();
            }
            if title != self.title {
                self.title = title.clone();
                self.send_event(EngineEvent::TitleChanged {
                    tab_id: self.tab_id,
                    title,
                });
            }
        }
        // Every full remote render reports the icon again; only a new one is fetched.
        if let Some(icon) = favicon.and_then(|f| Url::parse(&f).ok()) {
            if self.remote_favicon.as_ref() != Some(&icon) {
                self.remote_favicon = Some(icon.clone());
                let cancel = self.active_nav.as_ref().map(|a| a.cancel.clone()).unwrap_or_default();
                self.fetch_favicon(icon, &cancel);
            }
        }
    }

    /// Do a draw tick. This will be called based on the FPS that is requested
    #[allow(unreachable_code)] // cfg-conditional tile-cache returns make the display-list path unreachable for some feature combos
    async fn tick_draw(&mut self) -> anyhow::Result<()> {
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        self.apply_remote_document_meta();
        // Deferred scroll for a freshly committed document (history restore or URL fragment),
        // once it has laid out: page height and element positions are only known then. The
        // first dirty tick after `set_document` runs layout; this applies on the tick after
        // that and re-renders at the new offset.
        if self.pending_scroll.is_some() && self.context.page_height() > 0.0 {
            let target = match self.pending_scroll.take() {
                Some(PendingScroll::Offset(x, y)) => Some((x, y)),
                Some(PendingScroll::Fragment(f)) => self.context.fragment_target_y(&f).map(|y| (0, y.round() as i32)),
                None => None,
            };
            if let Some((x, y)) = target {
                self.apply_scroll(x, y);
            }
        }

        // Advance an in-flight smooth scroll: ease the engine scroll one step toward its target and
        // keep the frame loop alive (mark dirty) until it settles exactly on the target. Dormant
        // unless the scroll behavior is animated - `Instant` applies moves synchronously in the
        // MouseScroll handler, so `animating()` stays false there.
        if self.scroll.animating() {
            let now = std::time::Instant::now();
            let dt = self
                .scroll_anim_last
                .map(|t| now.duration_since(t).as_secs_f64())
                .unwrap_or(1.0 / self.runtime.fps.max(1) as f64);
            self.scroll_anim_last = Some(now);
            if let Some((x, y)) = self.scroll.tick(dt) {
                if x != self.scroll_x || y != self.scroll_y {
                    self.scroll_x = x;
                    self.scroll_y = y;
                    self.context.set_scroll(x as f64, y as f64);
                    self.runtime.dirty = true;
                }
            }
            if !self.scroll.animating() {
                self.scroll_anim_last = None;
            }
        }

        // A background media fetch (e.g. an image that started downloading during layout) landing
        // must wake the render loop even when nothing else changed, so the now-available image is
        // laid out and painted. This marks the render dirty under the hood.
        if self.context.poll_media_completed() {
            self.runtime.dirty = true;
        }

        // Out-of-process work landing - an image the renderer went without, a
        // scroll or hover pass, a renderer that died - must wake the loop too.
        #[cfg(all(feature = "process-isolation", target_os = "linux"))]
        {
            if let Some(pool) = self.zone_context.engine_context.renderer_pool.get() {
                pool.sweep_dead();
            }
            if self.context.poll_remote_passes() {
                self.runtime.dirty = true;
            }
            self.apply_remote_effects();
        }
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
            if let Some(rasterizer) = gosub_render_pipeline::rasterizer::downcast_rasterizer(
                render_backend.create_rasterizer(self.zone_context.font_system.clone()),
            ) {
                self.context
                    .set_rasterizer(rasterizer, render_backend.raster_strategy());
            }
        }

        // TileCache path - used by CPU-compositing rasterizing backends (Cairo, Skia).
        //
        // These backends don't need the display-list render pipeline: tiles are rasterized
        // during stages 1-6 and the host composites them directly. A scroll-only fast path
        // skips stages 1-6 when only the offset changed.
        //
        // Backends that composite to a GPU texture (Vello) still rasterize tiles, but fall
        // through to the display-list path below so the backend draws those tiles into a GPU
        // texture and the host presents a `WgpuTextureId` instead of compositing CPU tiles.
        //
        // DPR comes from the backend: every backend that honours it rasterizes at physical
        // pixels on a HiDPI host. Only the null backend stays at 1.
        let remote_render = self.context.remote_render_active();
        if remote_render
            || (render_backend.raster_strategy() != RasterStrategy::None && !render_backend.renders_to_gpu_texture())
        {
            let dpr = render_backend.device_pixel_ratio();
            let frame_started = std::time::Instant::now();

            // The host can change the DPR behind our back (page zoom writes the global atomic),
            // which invalidates every cached tile's pixel size. Do this before the scroll fast
            // path, which would otherwise hand back old tiles stamped with the new DPR.
            self.context.invalidate_raster_if_dpr_changed(dpr);

            // Scroll-only fast path: tiles are still valid, only the offset changed.
            if let Some(handle) = self.context.take_scroll_handle(dpr) {
                self.runtime.committed_scene_epoch = self.context.scene_epoch();
                self.submit_frame(handle);
                self.report_frame("scroll", frame_started);
                return Ok(());
            }

            // Full render: rebuild stages 1-6 only (no display list), then submit TileCache.
            self.context.set_viewport(self.desired_viewport);
            self.context.rebuild_pipeline_cache_if_needed();
            #[cfg(all(feature = "process-isolation", target_os = "linux"))]
            if let Some(error) = self.context.take_remote_failure() {
                let site = self
                    .current_url
                    .as_ref()
                    .map(crate::fork_server::site::site_of)
                    .unwrap_or_default();
                self.send_event(EngineEvent::RendererCrashed {
                    zone_id: self.zone_id,
                    site,
                    tabs: vec![self.tab_id],
                    error,
                });
            }
            let scene_epoch = self.context.scene_epoch();
            if let Some(handle) = self.context.tile_cache_handle(dpr) {
                self.runtime.committed_scene_epoch = scene_epoch;
                self.submit_frame(handle);
            }
            self.sink.inc_frame();
            self.report_frame("rebuild", frame_started);
            return Ok(());
        }

        // GPU scene path - backends that composite to a GPU texture (Vello).
        //
        // Skips tiling/rasterization/compositing: the engine builds one viewport-level paint
        // command list (stages 1–3 + paint), and the backend renders it into a GPU texture.
        // The host then presents the resulting `WgpuTextureId`. Scroll re-renders with a new
        // translate (no rebuild); only content/hover/size changes rebuild the command list.
        if render_backend.renders_to_gpu_texture() {
            // The viewport is CSS pixels, the texture is physical ones. Sizing the texture in
            // CSS pixels leaves the host to upscale it, which reads as blurry text rather than
            // as the scaling bug it is. The backend scales its scene to match (or, on the GPU
            // tile path, rasterizes its tiles at this ratio), so the page is re-rendered at
            // this resolution instead of stretched.
            let dpr = render_backend.device_pixel_ratio().max(1);
            let mut surface_size = self.desired_viewport.as_size();
            surface_size.width *= dpr;
            surface_size.height *= dpr;

            let surface_recreated = self.ensure_surface_tracked(render_backend.clone(), surface_size)?;
            self.context.set_viewport(self.desired_viewport);

            // Consolidated tile path (opt-in): rather than the one-shot whole-viewport scene, run
            // the SAME shared tile pipeline the CPU backends use (stages 1-6 → cached tiles). The
            // backend's rasterizer renders each tile into a GPU texture instead of CPU memory, and
            // `composite_tiles` blits the resident tiles into the surface. Same pipeline, only the
            // tile storage + compositor differ between CPU and GPU backends.
            if render_backend.gpu_tile_compositing() {
                // Resident tiles were rasterized at the DPR they were made at; page zoom (or a
                // move to another display) changes it under them, and they would be stretched.
                self.context.invalidate_raster_if_dpr_changed(dpr);
                {
                    // If `pipeline.rasterize` shows up here during a pure scroll, the page is being
                    // re-rasterized (it should not be - scroll only re-composites cached tiles).
                    let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::GpuTileRebuild);
                    self.context.rebuild_pipeline_cache_if_needed();
                }
                let scene_epoch = self.context.scene_epoch();
                if !surface_recreated && scene_epoch == self.runtime.committed_scene_epoch {
                    return Ok(());
                }
                if let Some(ref mut surf) = self.surface {
                    let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::GpuTileComposite);
                    let tiles = self.context.placed_gpu_tiles();
                    let vp = (self.desired_viewport.width, self.desired_viewport.height);
                    let (sx, sy) = self.context.scroll_xy();
                    let page_height = self.context.page_height() as f32;
                    match render_backend.composite_tiles(surf.as_mut(), &tiles, vp, (sx as f32, sy as f32), page_height)
                    {
                        Ok(()) => match render_backend.external_handle(surf.as_mut()) {
                            Ok(handle) => {
                                self.runtime.committed_scene_epoch = scene_epoch;
                                self.submit_frame(handle);
                            }
                            Err(e) => log::warn!("[tick_draw] gpu-tile external_handle error: {e}"),
                        },
                        Err(e) => log::warn!("[tick_draw] composite_tiles error: {e}"),
                    }
                }
                self.sink.inc_frame();
                return Ok(());
            }

            self.context.rebuild_scene_cache_if_needed();

            let scene_epoch = self.context.scene_epoch();
            if !surface_recreated && scene_epoch == self.runtime.committed_scene_epoch {
                return Ok(());
            }

            if let Some(ref mut surf) = self.surface {
                render_backend.render(&mut self.context, surf.as_mut())?;
                match render_backend.external_handle(surf.as_mut()) {
                    Ok(handle) => {
                        self.runtime.committed_scene_epoch = scene_epoch;
                        self.submit_frame(handle);
                    }
                    Err(e) => log::warn!("[tick_draw] gpu external_handle error: {e}"),
                }
            }
            self.sink.inc_frame();
            return Ok(());
        }

        // Display-list render path: reached only by the null backend (no rasterizer).

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
                            _ => "Other".to_string(),
                        }
                    );
                    self.runtime.committed_scene_epoch = scene_epoch;
                    self.submit_frame(handle);
                }
                Err(e) => {
                    log::warn!("[tick_draw] external_handle error: {e}");
                }
            }
        }
        let render_ms = render_start.elapsed().as_millis();

        self.sink.inc_frame();

        // First frame this navigation has actually produced. `tick_draw` runs every frame,
        // so the flag is what makes this a mark ("when did the page first appear") rather
        // than a per-frame sample; it is cleared when a navigation starts.
        if !self.first_paint_marked {
            if let Some(scope) = self.timing_scope {
                gosub_shared::timing::mark_in(scope, gosub_shared::timing::Timing::PageFirstPaint, None);
                self.first_paint_marked = true;
            }
        }

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
    ///
    /// Zero-sized viewports are ignored rather than applied: they are never a state worth
    /// rendering (a minimized or not-yet-allocated host window reports one), and applying one
    /// would drop the whole tile cache and re-layout the page at `MAX_CONTENT`. Keeping the last
    /// good viewport means the tab still holds a valid frame when the host comes back.
    pub fn set_viewport(&mut self, vp: Viewport) {
        if vp.width == 0 || vp.height == 0 {
            log::debug!("[render] ignoring zero-sized viewport {vp:?} for tab {:?}", self.tab_id);
            return;
        }
        // Already at the viewport we want, then we can skip
        if vp == self.desired_viewport {
            return;
        }
        self.desired_viewport = vp;
        self.state = TabState::PendingRendering(self.desired_viewport);
        self.runtime.dirty = true;
    }

    /// Bind local+session storage handles into the underlying browsing context.
    /// Call this after creating the tab or when the zone’s storage changes.
    pub fn bind_storage(&mut self, storage: StorageHandles) {
        self.context.bind_storage(storage.local, storage.session);
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
                "Cancelling active navigation for tab {:?} nav {:?}",
                self.tab_id,
                active.nav_id
            );
            active.cancel.cancel();
            // Whatever that navigation preloaded is now for a page nobody will see. Its own
            // entries only: the hand-off is process-wide, and every other tab's preloads are
            // in there too.
            gosub_shared::subresource::clear_scope(active.nav_id.as_scope());
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
                        error: LoadError::InvalidUrl { message: e.to_string() },
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
                        error: LoadError::Io {
                            message: format!("{e:#}"),
                        },
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
    /// Accepting an offer places the already-fetched body; it never re-requests the URL.
    mod spooled_downloads {
        use super::super::place_spooled_download;
        use std::io::Write;

        fn spooled(contents: &[u8]) -> tempfile::TempPath {
            let mut f = tempfile::NamedTempFile::new().unwrap();
            f.write_all(contents).unwrap();
            f.flush().unwrap();
            f.into_temp_path()
        }

        #[test]
        fn places_the_body_and_reports_its_size() {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("file.bin");
            let src = spooled(b"downloaded bytes");
            let src_path = src.to_path_buf();

            let n = place_spooled_download(src, &target).unwrap();

            assert_eq!(n, 16);
            assert_eq!(std::fs::read(&target).unwrap(), b"downloaded bytes");
            assert!(!src_path.exists(), "spooled file should not be left behind");
        }

        #[test]
        fn creates_missing_parent_directories() {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("nested/deeper/file.bin");
            place_spooled_download(spooled(b"x"), &target).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"x");
        }

        #[test]
        fn overwrites_an_existing_target() {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join("file.bin");
            std::fs::write(&target, b"stale contents").unwrap();
            place_spooled_download(spooled(b"new"), &target).unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"new");
        }

        /// The offer holds a `TempPath`, so an offer the embedder never accepts must not
        /// leave its body in the temp directory.
        #[test]
        fn unaccepted_offer_deletes_its_spooled_body() {
            let src = spooled(b"never accepted");
            let path = src.to_path_buf();
            assert!(path.exists());
            drop(src);
            assert!(!path.exists());
        }
    }

    use crate::net::SharedBody;
    use bytes::Bytes;
    use futures_util::TryStreamExt;

    /// `about_blank()` keeps an `unwrap`, so the claim that it cannot fail is checked rather
    /// than asserted in a comment.
    #[test]
    fn about_blank_parses() {
        assert_eq!(super::about_blank().as_str(), "about:blank");
    }

    mod favicon_url {
        use crate::html::DefaultRenderConfig;
        use url::Url;

        fn resolve(html: &str, base: &str) -> Option<String> {
            let doc = gosub_html5::html_compile::<DefaultRenderConfig>(html);
            let base = Url::parse(base).unwrap();
            crate::html::favicon_url::<DefaultRenderConfig>(&doc, &base).map(|u| u.to_string())
        }

        #[test]
        fn link_rel_icon_wins_and_resolves_relative() {
            let html = r#"<html><head><link rel="icon" href="img/fav.png"></head><body></body></html>"#;
            assert_eq!(
                resolve(html, "https://example.com/dir/page.html").as_deref(),
                Some("https://example.com/dir/img/fav.png")
            );
        }

        /// A `rel` token whose 16th byte falls inside a multi-byte character is
        /// not an icon, and must not panic the tab working that out.
        #[test]
        fn a_multibyte_rel_token_is_no_icon_and_no_panic() {
            let html = r#"<html><head><link rel="apple-touch-icoé" href="/a.png"></head></html>"#;
            assert_eq!(
                resolve(html, "https://example.com/").as_deref(),
                Some("https://example.com/favicon.ico")
            );
        }

        #[test]
        fn shortcut_icon_and_apple_touch_icon_count() {
            let html = r#"<html><head><link rel="Shortcut Icon" href="/a.ico"></head></html>"#;
            assert_eq!(
                resolve(html, "https://example.com/").as_deref(),
                Some("https://example.com/a.ico")
            );
            let html = r#"<html><head><link rel="apple-touch-icon" href="/t.png"></head></html>"#;
            assert_eq!(
                resolve(html, "https://example.com/").as_deref(),
                Some("https://example.com/t.png")
            );
        }

        #[test]
        fn stylesheet_links_are_ignored_and_fallback_is_well_known() {
            let html = r#"<html><head><link rel="stylesheet" href="/s.css"></head></html>"#;
            assert_eq!(
                resolve(html, "https://example.com/deep/path").as_deref(),
                Some("https://example.com/favicon.ico")
            );
        }

        #[test]
        fn no_fallback_for_non_http_documents() {
            assert_eq!(resolve("<html></html>", "gosub://home"), None);
        }
    }

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
