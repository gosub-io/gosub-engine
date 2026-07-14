use crate::cookies::SameSiteContext;
use crate::engine::errors::NavigationError;
use crate::engine::events::{EngineEvent, NavigationEvent};
use crate::engine::resource_pipeline::ResourcePipelines;
use crate::engine::types::{NavigationId, RequestId};
use crate::engine::{BrowsingContext, UaPolicy};
use crate::events::{IoCommand, TabCommand};
use crate::html::RenderConfiguration;
use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{FetchRequest, FetchResult, Initiator, NetError, Priority, ResourceKind};
use crate::net::{route_response_for, submit_to_io, RequestDestination, RoutedOutcome};
use crate::storage::types::compute_partition_key;
use crate::storage::StorageHandles;
use crate::tab::scroll::{default_text_scroll, ScrollState};
use crate::tab::services::EffectiveTabServices;
use crate::tab::state::{TabRuntime, TabState};
use crate::tab::{TabId, TabSink};
use crate::util::spawn_named;
use crate::zone::{ZoneContext, ZoneId};
use anyhow::{anyhow, Context};
use gosub_render_pipeline::rasterizer::RasterStrategy;
use gosub_render_pipeline::render::backend::{
    CompositorSink, ErasedSurface, PresentMode, RenderBackend, SurfaceSize,
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
pub enum NavigationResult<C: RenderConfiguration> {
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
    /// Keeps track of the tab worker runtime data
    pub(crate) runtime: TabRuntime,
    /// Current in-flight navigation (if any)
    load: Option<NavJoin<C>>,
    /// Current active navigation (if any)
    active_nav: Option<ActiveNav>,
}

/// Whether a CSS `unicode-range` descriptor (e.g. `"U+0000-00FF, U+0131"`) includes the
/// Basic-Latin letter `U+0041` ('A') — our proxy for "covers Latin-script text".
fn unicode_range_covers_basic_latin(range: &str) -> bool {
    const TARGET: u32 = 0x41; // 'A'
    for token in range.split([',', ' ', '\t', '\n', '\r']).filter(|t| !t.is_empty()) {
        let Some(hex) = token
            .trim()
            .strip_prefix("U+")
            .or_else(|| token.trim().strip_prefix("u+"))
        else {
            continue;
        };
        let (lo, hi) = match hex.split_once('-') {
            Some((a, b)) => (parse_hex_bound(a, false), parse_hex_bound(b, true)),
            None => (parse_hex_bound(hex, false), parse_hex_bound(hex, true)),
        };
        if let (Some(lo), Some(hi)) = (lo, hi) {
            if lo <= TARGET && TARGET <= hi {
                return true;
            }
        }
    }
    false
}

/// Unwrap a downloaded web-font payload into raw SFNT bytes the font backends can decode.
///
/// WOFF2 (magic `wOF2`) is a Brotli-compressed wrapper around an OpenType/TrueType font,
/// with the `glyf`/`loca` tables stored in a transformed form. Skia and fontconfig don't
/// decode it (e.g. Google Fonts serves WOFF2 to modern UAs like ours), so we decompress it
/// to a flat SFNT here. Bare SFNT (`OTTO`/`true`/`ttcf`/`0x00010000`) and anything we don't
/// recognise are returned unchanged — including WOFF1, which the backends already handle.
/// On a decode error we log and return the original bytes so the subsequent `register_font`
/// surfaces a single, consistent failure path.
fn decode_web_font(bytes: Vec<u8>, font_url: &Url) -> Vec<u8> {
    const WOFF2_MAGIC: &[u8; 4] = b"wOF2";
    if bytes.len() < 4 || &bytes[0..4] != WOFF2_MAGIC {
        return bytes;
    }
    match woff2_to_sfnt(&bytes) {
        Ok(sfnt) => {
            log::debug!(
                "Decoded WOFF2 web font from {font_url} ({} → {} bytes)",
                bytes.len(),
                sfnt.len()
            );
            sfnt
        }
        Err(e) => {
            log::warn!("Failed to decode WOFF2 web font from {font_url}: {e}");
            bytes
        }
    }
}

/// Decompress a WOFF2 font to a flat SFNT (TTF/OTF) byte buffer. allsorts handles the Brotli
/// decompression and the `glyf`/`loca` transform reconstruction; we then re-assemble the
/// reconstructed tables into the on-disk SFNT layout (offset table + table directory + 4-byte
/// aligned table data) that font backends expect.
fn woff2_to_sfnt(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use allsorts::binary::read::ReadScope;
    use allsorts::woff2::Woff2Font;

    let font = ReadScope::new(bytes)
        .read::<Woff2Font<'_>>()
        .map_err(|e| format!("parse: {e:?}"))?;
    let sfnt_version = font.flavor();
    let tables = font
        .table_provider(0)
        .map_err(|e| format!("reconstruct: {e:?}"))?
        .into_tables();

    Ok(assemble_sfnt(sfnt_version, tables))
}

/// Pack a set of font tables into an SFNT byte buffer per the OpenType spec: a 12-byte offset
/// table, a 16-byte directory entry per table (sorted by tag), then each table's data padded to
/// a 4-byte boundary. Per-table checksums are computed; the `head` table's `checkSumAdjustment`
/// is left as-is (font backends parse without validating it).
fn assemble_sfnt(sfnt_version: u32, tables: std::collections::HashMap<u32, Box<[u8]>>) -> Vec<u8> {
    let mut entries: Vec<(u32, Box<[u8]>)> = tables.into_iter().collect();
    entries.sort_by_key(|(tag, _)| *tag);
    let num_tables = entries.len() as u16;

    // Binary-search hint fields: largest power of two <= num_tables.
    let mut entry_selector = 0u16;
    while (1u16 << (entry_selector + 1)) <= num_tables {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = num_tables.wrapping_mul(16).wrapping_sub(search_range);

    let mut directory = Vec::with_capacity(16 * entries.len());
    let mut data = Vec::new();
    let mut offset = 12 + 16 * entries.len();
    for (tag, table) in &entries {
        directory.extend_from_slice(&tag.to_be_bytes());
        directory.extend_from_slice(&sfnt_table_checksum(table).to_be_bytes());
        directory.extend_from_slice(&(offset as u32).to_be_bytes());
        directory.extend_from_slice(&(table.len() as u32).to_be_bytes());
        data.extend_from_slice(table);
        while data.len() % 4 != 0 {
            data.push(0);
        }
        offset += (table.len() + 3) & !3;
    }

    let mut out = Vec::with_capacity(12 + directory.len() + data.len());
    out.extend_from_slice(&sfnt_version.to_be_bytes());
    out.extend_from_slice(&num_tables.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&entry_selector.to_be_bytes());
    out.extend_from_slice(&range_shift.to_be_bytes());
    out.extend_from_slice(&directory);
    out.extend_from_slice(&data);
    out
}

/// SFNT table checksum: the sum of the table's contents read as big-endian `u32`s, with the
/// final partial word zero-padded, in wrapping (mod 2^32) arithmetic.
fn sfnt_table_checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

/// Parse a `unicode-range` hex bound, expanding `?` wildcards to `0` (low bound) or `F`
/// (high bound), e.g. `U+00??` → `0x0000..=0x00FF`.
fn parse_hex_bound(s: &str, high: bool) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let filled: String = s
        .chars()
        .map(|c| {
            if c == '?' {
                if high {
                    'F'
                } else {
                    '0'
                }
            } else {
                c
            }
        })
        .collect();
    u32::from_str_radix(&filled, 16).ok()
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
        let context = BrowsingContext::new(config_store.clone());
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

    /// Fetch and register any `@font-face` web fonts declared in the document's stylesheets
    /// so the first layout/paint can use them. Runs once per navigation, before the first
    /// render, and deduplicates by resolved font URL. Fetches are synchronous (blocking this
    /// worker briefly during initial load); each face is registered under its CSS family so
    /// the font system selects the right weight/style from the font's own metadata.
    fn load_web_fonts(&self, doc: &C::Document, base_url: &Url) {
        use gosub_interface::css3::CssStylesheet as _;
        use gosub_interface::document::Document as _;
        use gosub_interface::font_system::FontSystem as _;

        let mut fetched: std::collections::HashSet<String> = std::collections::HashSet::new();
        for sheet in doc.stylesheets() {
            let sheet_url = Url::parse(sheet.url()).ok();
            for (family, sources, unicode_range) in sheet.font_faces() {
                // Google-style web fonts split a family into many `unicode-range` subsets
                // (latin, cyrillic, greek, …). We don't do per-glyph subset fallback, so
                // register only subsets covering Basic Latin (and ranges with no descriptor),
                // which covers Latin-script content without piling unusable subsets onto the
                // same family.
                if let Some(range) = &unicode_range {
                    if !unicode_range_covers_basic_latin(range) {
                        continue;
                    }
                }
                for src in &sources {
                    let resolved = sheet_url
                        .as_ref()
                        .unwrap_or(base_url)
                        .join(src)
                        .or_else(|_| base_url.join(src));
                    let Ok(font_url) = resolved else { continue };
                    if !fetched.insert(font_url.to_string()) {
                        break; // this exact font file is already registered
                    }
                    match gosub_sonar::net::simple::sync_fetch(&font_url) {
                        Ok(resp) if resp.status == 200 && !resp.body.is_empty() => {
                            // Web fonts are commonly served as WOFF2 (e.g. Google Fonts content-
                            // negotiates WOFF2 for modern UAs like ours). The font backends
                            // (Skia/fontconfig) only decode raw SFNT (TTF/OTF), so unwrap WOFF2
                            // to TTF first. Other formats pass through unchanged.
                            let font_bytes = decode_web_font(resp.body, &font_url);
                            match self
                                .zone_context
                                .font_system
                                .lock()
                                .register_font(font_bytes, Some(&family))
                            {
                                Ok(()) => {
                                    log::debug!("Registered web font '{family}' from {font_url}");
                                    break; // family face loaded; skip remaining sources
                                }
                                Err(e) => log::warn!("Failed to register web font '{family}': {e:?}"),
                            }
                        }
                        Ok(resp) => log::warn!("Web font fetch {font_url} returned status {}", resp.status),
                        Err(e) => log::warn!("Web font fetch {font_url} failed: {e}"),
                    }
                }
            }
        }
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
                self.load_web_fonts(&doc, &final_url);
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
                        (ph - self.desired_viewport.height as f64).max(0.0)
                    } else {
                        f64::MAX
                    }
                };

                match self.scroll.scroll_by(delta_x as f64, delta_y as f64, f64::MAX, max_y) {
                    // Instant behavior: apply the new offset now and keep the immediate-submit fast
                    // path (avoids up to 1/fps of latency per scroll event).
                    Some((x, y)) => {
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
                            if let Some(handle) = self.context.take_scroll_handle(dpr) {
                                self.runtime.committed_scene_epoch = self.context.scene_epoch();
                                self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
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
        self.scroll.reset(0.0, 0.0);
        self.scroll_anim_last = None;
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
        if let Some(langs) = &self.services.accept_language {
            if let Ok(val) = langs.parse() {
                fetch_headers.insert(http::header::ACCEPT_LANGUAGE, val);
            }
        }

        let req_id = RequestId::new();
        REF_REGISTRY.register_request(req_id, ResourceKind::Document, Initiator::Navigation);
        let req = FetchRequest::builder(Method::GET, url.clone())
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
            .with_auto_decode(true)
            .build();

        let (tx_done, rx_done) = oneshot::channel::<NavigationResult<C>>();

        let tab_id = self.tab_id;
        let zone_id = self.zone_id;
        let io_tx = self.zone_context.io_tx.clone();
        let event_tx = self.zone_context.event_tx.clone();
        let cookie_jar = self.services.cookie_jar.clone();
        let accept_language = self.services.accept_language.clone();

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
                allow_download_without_user_activation: false,
            };

            let mut hooks = ResourcePipelines::<C>::new(zone_id, io_tx.clone(), accept_language.clone());

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
                // Subresource outcomes need no main-frame navigation handling.
                Ok(
                    RoutedOutcome::CssLoaded(_)
                    | RoutedOutcome::ScriptExecuted(_)
                    | RoutedOutcome::ImageDecoded(_)
                    | RoutedOutcome::FontLoaded(_),
                ) => {
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
                            error: Arc::new(anyhow!("Reason: {}", reason)),
                        },
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
        // Advance an in-flight smooth scroll: ease the engine scroll one step toward its target and
        // keep the frame loop alive (mark dirty) until it settles exactly on the target. Dormant
        // unless the scroll behavior is animated — `Instant` applies moves synchronously in the
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

        // TileCache path — used by CPU-compositing rasterizing backends (Cairo, Skia).
        //
        // These backends don't need the display-list render pipeline: tiles are rasterized
        // during stages 1-6 and the host composites them directly. A scroll-only fast path
        // skips stages 1-6 when only the offset changed.
        //
        // Backends that composite to a GPU texture (Vello) still rasterize tiles, but fall
        // through to the display-list path below so the backend draws those tiles into a GPU
        // texture and the host presents a `WgpuTextureId` instead of compositing CPU tiles.
        //
        // DPR comes from the backend: Cairo rasterizes at physical pixels (DPR > 1 on HiDPI);
        // Skia and Vello rasterize at CSS pixels (DPR = 1).
        if render_backend.raster_strategy() != RasterStrategy::None && !render_backend.renders_to_gpu_texture() {
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

        // GPU scene path — backends that composite to a GPU texture (Vello).
        //
        // Skips tiling/rasterization/compositing: the engine builds one viewport-level paint
        // command list (stages 1–3 + paint), and the backend renders it into a GPU texture.
        // The host then presents the resulting `WgpuTextureId`. Scroll re-renders with a new
        // translate (no rebuild); only content/hover/size changes rebuild the command list.
        if render_backend.renders_to_gpu_texture() {
            let surface_recreated =
                self.ensure_surface_tracked(render_backend.clone(), self.desired_viewport.as_size())?;
            self.context.set_viewport(self.desired_viewport);

            // Consolidated tile path (opt-in): rather than the one-shot whole-viewport scene, run
            // the SAME shared tile pipeline the CPU backends use (stages 1-6 → cached tiles). The
            // backend's rasterizer renders each tile into a GPU texture instead of CPU memory, and
            // `composite_tiles` blits the resident tiles into the surface. Same pipeline, only the
            // tile storage + compositor differ between CPU and GPU backends.
            if render_backend.gpu_tile_compositing() {
                {
                    // If `pipeline.rasterize` shows up here during a pure scroll, the page is being
                    // re-rasterized (it should not be — scroll only re-composites cached tiles).
                    let _t = gosub_shared::timing_guard!("gputile.rebuild");
                    self.context.rebuild_pipeline_cache_if_needed();
                }
                let scene_epoch = self.context.scene_epoch();
                if !surface_recreated && scene_epoch == self.runtime.committed_scene_epoch {
                    return Ok(());
                }
                if let Some(ref mut surf) = self.surface {
                    let _t = gosub_shared::timing_guard!("gputile.composite");
                    let tiles = self.context.placed_gpu_tiles();
                    let vp = (self.desired_viewport.width, self.desired_viewport.height);
                    let (sx, sy) = self.context.scroll_xy();
                    let page_height = self.context.page_height() as f32;
                    match render_backend.composite_tiles(surf.as_mut(), &tiles, vp, (sx as f32, sy as f32), page_height)
                    {
                        Ok(()) => match render_backend.external_handle(surf.as_mut()) {
                            Ok(handle) => {
                                self.runtime.committed_scene_epoch = scene_epoch;
                                self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
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
                        self.zone_context.compositor.write().submit_frame(self.tab_id, handle);
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

    /// Verify `decode_web_font` turns a real WOFF2 payload into an SFNT the font stack can
    /// parse. Reads the fixture path from `GOSUB_WOFF2_FIXTURE` so we neither hit the network
    /// nor commit a binary font; skips when unset.
    #[test]
    fn decode_web_font_woff2_roundtrips_to_sfnt() {
        let Ok(path) = std::env::var("GOSUB_WOFF2_FIXTURE") else {
            eprintln!("skipping: set GOSUB_WOFF2_FIXTURE to a .woff2 file to run");
            return;
        };
        let woff2 = std::fs::read(&path).expect("read fixture");
        assert_eq!(&woff2[0..4], b"wOF2", "fixture must be WOFF2");

        let url = url::Url::parse("https://example.test/font.woff2").unwrap();
        let sfnt = super::decode_web_font(woff2, &url);

        // Output must be a different, valid SFNT (TrueType `0x00010000` or OpenType `OTTO`).
        let magic = u32::from_be_bytes([sfnt[0], sfnt[1], sfnt[2], sfnt[3]]);
        assert!(magic == 0x0001_0000 || magic == 0x4F54_544F, "not SFNT: {magic:#010x}");

        // It must re-parse and expose the core tables a backend reads.
        use allsorts::binary::read::ReadScope;
        use allsorts::font_data::FontData;
        use allsorts::tables::FontTableProvider;
        let font = ReadScope::new(&sfnt).read::<FontData<'_>>().expect("parse SFNT");
        let provider = font.table_provider(0).expect("table provider");
        for tag in [allsorts::tag::HEAD, allsorts::tag::CMAP, allsorts::tag::GLYF] {
            assert!(provider.has_table(tag), "missing table {tag:#010x}");
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
