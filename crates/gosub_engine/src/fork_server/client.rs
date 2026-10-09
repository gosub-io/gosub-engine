//! The broker's side of the fork server: spawn it, learn its confinement
//! tier, ask it to fork.

use crate::fork_server::protocol::{
    ConfinementTier, FromForkServer, FromRenderer, HitRegion, MediaPrefs, PageSummary, ResourceReply, TileHeader,
    ToForkServer, ToRenderer, MAX_HIT_TEXT,
};
use crate::fork_server::protocol::{Effect, InputEvent, WireRect, MAX_EFFECTS};
use crate::net::resource_loader::{LoadError, LoadedResource};
use crate::net::types::ResourceKind;
use gosub_ipc::Endpoint;
use std::time::Duration;

/// The argv role name the broker re-execs itself with.
pub const FORK_SERVER_ROLE: &str = "fork-server";

/// Committed memory a renderer process may hold (`RLIMIT_DATA`).
pub const RENDERER_DATA_LIMIT: u64 = 1024 * 1024 * 1024;
/// Tasks a renderer may have at once (`pids.max`): it never forks and
/// rasterizes sequentially, so this is the fork-bomb bound, not a budget.
pub const RENDERER_MAX_TASKS: u32 = 256;

/// The user's media preferences as last set by [`set_media_prefs`]. The
/// in-process pipeline treats them as process-wide (one colour scheme per
/// engine, like the device-pixel ratio); every render request carries them to
/// the renderer, which has no settings of its own to read.
static MEDIA_PREFS: parking_lot::Mutex<MediaPrefs> = parking_lot::Mutex::new(MediaPrefs {
    prefers_dark: false,
    prefers_reduced_motion: false,
});

/// Record the preferences a render request should carry; the tab sets them
/// from its settings before each remote render.
pub fn set_media_prefs(prefs: MediaPrefs) {
    *MEDIA_PREFS.lock() = prefs;
}

/// What the next render request carries - see [`set_media_prefs`].
pub fn media_prefs() -> MediaPrefs {
    *MEDIA_PREFS.lock()
}

/// Remove the scratch directory a child claimed (`claim_scratch_dir`), once it
/// has exited. The child cannot: its lockdown has no `unlinkat`.
pub(crate) fn remove_scratch_dir(role: &str, pid: u32) {
    let dir = std::env::temp_dir().join(format!("gosub-{role}-scratch-{pid}"));
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::debug!("could not remove {}: {e}", dir.display());
        }
    }
}

/// How long to wait for `Ready`. Spawn plus font warm-up: the slowest measured
/// preparation (full warm-up on a font-heavy host) is well under a second, so
/// tens of seconds means a process that is not a fork server.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// How long any later request may take. A fork plus one shape is milliseconds.
/// Not a render: see [`RENDER_GAP`].
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest a render exchange may go quiet between two messages, for a
/// forked and an exec'd renderer alike. It bounds *gaps*, not the whole
/// render (a page streams a message per tile), and a heavy layout is one
/// long gap, so it is well above the control-message [`REPLY_TIMEOUT`].
pub(crate) const RENDER_GAP: Duration = Duration::from_secs(30);

/// How long a resident renderer may take to answer one request. Unlike the
/// fork server's control replies this covers whole renders, and a heavy
/// page's layout alone runs two-digit seconds today - the timeout is for a
/// *wedged* renderer, and declaring a merely slow one dead kills it for
/// nothing (the abandoned link reads as EOF in the child).
const RESIDENT_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Bounds on one render exchange, so a renderer cannot hold the tab thread
/// or fill the broker's memory by talking forever. Generous: a heavy page
/// stays far below every one of them.
const MAX_EXCHANGE_MESSAGES: usize = 50_000;
const MAX_EXCHANGE_RESOURCES: usize = 2_000;
const MAX_EXCHANGE_TILE_BYTES: usize = 512 * 1024 * 1024;
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(600);
/// Longest title kept; `link`/`image`/favicon URLs are bounded by
/// [`MAX_HIT_TEXT`] and dropped whole past it.
const MAX_TITLE: usize = 1024;
/// A form body an input pass may hand over. A longer one drops the whole
/// submission: cut, it would be a different form.
const MAX_FORM_BODY: usize = 1024 * 1024;
/// Text a page may put on the clipboard in one go.
const MAX_CLIPBOARD_TEXT: usize = 1024 * 1024;
const MAX_LAYER_ORDER: usize = 100_000;
const MAX_TIMINGS: usize = 64;
const MAX_TIMING_NAME: usize = 64;

/// What answers a renderer's subresource requests during an exchange: the
/// broker's loader, plus - for a tab - a cache that lets an image request be
/// answered at once and fetched in the background.
pub trait RenderResources {
    fn load(&self, url: &url::Url, kind: ResourceKind) -> Result<LoadedResource, LoadError>;
    /// A resource the render can do without for now (an image). The default
    /// fetches it anyway - correct, just not asynchronous.
    fn load_deferred(&self, url: &url::Url, kind: ResourceKind) -> Result<LoadedResource, LoadError> {
        self.load(url, kind)
    }
}

impl<T: crate::net::resource_loader::ResourceLoader + ?Sized> RenderResources for T {
    fn load(&self, url: &url::Url, kind: ResourceKind) -> Result<LoadedResource, LoadError> {
        crate::net::resource_loader::ResourceLoader::load(self, url, kind)
    }
}

/// Subresources fetched on a tab's behalf, images asynchronously.
pub struct TabResources {
    pub loader: std::sync::Arc<dyn crate::net::resource_loader::ResourceLoader>,
    pub media: std::sync::Arc<RemoteMediaCache>,
}

impl RenderResources for TabResources {
    fn load(&self, url: &url::Url, kind: ResourceKind) -> Result<LoadedResource, LoadError> {
        self.loader.load(url, kind)
    }

    fn load_deferred(&self, url: &url::Url, kind: ResourceKind) -> Result<LoadedResource, LoadError> {
        self.media
            .lookup_or_fetch(url, kind, std::sync::Arc::clone(&self.loader))
    }
}

/// Encoded bytes this cache keeps per tab; past it the oldest entries go and
/// the renderer asks for them again.
const MEDIA_CACHE_BUDGET: usize = 64 * 1024 * 1024;
/// Fetch threads per tab. The rest queue; the renderer re-asks after every
/// completion anyway.
const MAX_MEDIA_FETCHERS: usize = 6;
/// Entries this cache keeps per tab, failed and empty loads included: they
/// cost no budget bytes, so the byte budget alone never lets them go.
const MAX_MEDIA_ENTRIES: usize = 4096;
/// Fetches waiting for a fetcher thread. Past it an image fails outright (a
/// placeholder) rather than lengthening the queue.
const MAX_MEDIA_QUEUE: usize = 4096;

type MediaLoader = std::sync::Arc<dyn crate::net::resource_loader::ResourceLoader>;
/// One queued fetch: the URL, what it is for, and the loader to fetch it with.
type MediaJob = (url::Url, ResourceKind, MediaLoader);

#[derive(Default)]
struct MediaEntries {
    by_url: std::collections::HashMap<String, Result<LoadedResource, String>>,
    /// Insertion order, for eviction.
    order: std::collections::VecDeque<String>,
    bytes: usize,
}

impl MediaEntries {
    fn insert(&mut self, key: String, fetched: Result<LoadedResource, String>) {
        // A body the budget can never hold would be evicted as it lands, and
        // the renderer, asking again, would refetch it forever: it is a failed
        // load instead, which the renderer shows as a placeholder.
        let fetched = match fetched {
            Ok(r) if r.body.len() > MEDIA_CACHE_BUDGET => Err(format!(
                "image of {} bytes exceeds the per-tab media budget",
                r.body.len()
            )),
            other => other,
        };
        // A key fetched twice (a lookup racing the in-flight check) replaces
        // its entry rather than counting its bytes and its place twice.
        if let Some(old) = self.by_url.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.map_or(0, |r| r.body.len()));
            self.order.retain(|k| k != &key);
        }
        self.bytes += fetched.as_ref().map_or(0, |r| r.body.len());
        self.order.push_back(key.clone());
        self.by_url.insert(key.clone(), fetched);
        // Oldest first, but never the entry just stored: the renderer has not
        // had it yet.
        while self.bytes > MEDIA_CACHE_BUDGET || self.by_url.len() > MAX_MEDIA_ENTRIES {
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            if oldest == key {
                break;
            }
            self.order.pop_front();
            if let Some(gone) = self.by_url.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(gone.map_or(0, |r| r.body.len()));
            }
        }
    }
}

#[derive(Default)]
struct MediaQueue {
    waiting: std::collections::VecDeque<MediaJob>,
    fetchers: usize,
}

/// Images a renderer asked for on a tab's behalf: what has arrived, and what
/// is still on its way. A miss queues the fetch (a few threads work the
/// queue) and answers [`LoadError::Pending`]; when the bytes land, `completed`
/// rises and the tab renders again, this time finding them here.
#[derive(Default)]
pub struct RemoteMediaCache {
    entries: parking_lot::Mutex<MediaEntries>,
    in_flight: parking_lot::Mutex<std::collections::HashSet<String>>,
    queue: parking_lot::Mutex<MediaQueue>,
    completed: std::sync::atomic::AtomicBool,
}

impl RemoteMediaCache {
    pub fn lookup_or_fetch(
        self: &std::sync::Arc<Self>,
        url: &url::Url,
        kind: ResourceKind,
        loader: MediaLoader,
    ) -> Result<LoadedResource, LoadError> {
        let key = url.to_string();
        if let Some(entry) = self.entries.lock().by_url.get(&key) {
            return entry.clone().map_err(LoadError::Failed);
        }
        if !self.in_flight.lock().insert(key.clone()) {
            return Err(LoadError::Pending);
        }
        // Start a fetcher or queue for one, decided under the queue lock so a
        // fetcher finishing right now cannot miss what was just queued.
        let mut queue = self.queue.lock();
        if queue.fetchers >= MAX_MEDIA_FETCHERS {
            if queue.waiting.len() >= MAX_MEDIA_QUEUE {
                drop(queue);
                self.in_flight.lock().remove(&key);
                return Err(LoadError::Failed("too many images waiting to load".into()));
            }
            queue.waiting.push_back((url.clone(), kind, loader));
            return Err(LoadError::Pending);
        }
        queue.fetchers += 1;
        drop(queue);
        let cache = std::sync::Arc::clone(self);
        let first = (url.clone(), kind, loader);
        let spawned = std::thread::Builder::new()
            .name("gosub-remote-media".into())
            .spawn(move || cache.work(first));
        if spawned.is_err() {
            self.queue.lock().fetchers -= 1;
            self.in_flight.lock().remove(&key);
            return Err(LoadError::Failed("could not start the image fetch".into()));
        }
        Err(LoadError::Pending)
    }

    /// One fetcher thread: the job it was started for, then the queue until
    /// it is empty.
    fn work(&self, first: MediaJob) {
        let mut next = Some(first);
        while let Some((url, kind, loader)) = next.take() {
            let fetched = loader.load(&url, kind).map_err(|e| e.to_string());
            self.entries.lock().insert(url.to_string(), fetched);
            self.in_flight.lock().remove(url.as_str());
            self.completed.store(true, std::sync::atomic::Ordering::Release);
            let mut queue = self.queue.lock();
            next = queue.waiting.pop_front();
            if next.is_none() {
                queue.fetchers -= 1;
            }
        }
    }

    /// Whether an image landed since the last call - the tab should render
    /// again to pick it up.
    pub fn take_completed(&self) -> bool {
        self.completed.swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Forget everything (a new document). Fetches already running finish
    /// into the new page's cache, which is harmless.
    pub fn clear(&self) {
        *self.entries.lock() = MediaEntries::default();
        self.in_flight.lock().clear();
        self.queue.lock().waiting.clear();
        self.completed.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// One step of a render exchange, whichever peer is speaking.
pub(crate) enum RenderEvent {
    NeedResource {
        url: String,
        kind: ResourceKind,
        deferred: bool,
    },
    Tile(TileHeader),
    TileUnchanged(TileHeader),
    Rendered {
        summary: PageSummary,
        hit_regions: Vec<HitRegion>,
        effects: Vec<Effect>,
    },
    Evict(Vec<u64>),
    Refused(String),
}

/// A peer's render-exchange dialect: the fork server relays its forked
/// child's stream wrapped in [`FromForkServer`] and wants resources wrapped
/// back; a resident renderer (and an exec'd one) speaks [`FromRenderer`]
/// directly and its loader reads a bare [`ResourceReply`].
pub(crate) trait RenderStream: serde::de::DeserializeOwned + std::fmt::Debug {
    fn into_event(self) -> anyhow::Result<RenderEvent>;
    fn send_resource(link: &mut Endpoint, reply: ResourceReply) -> std::io::Result<()>;
}

impl RenderStream for FromForkServer {
    fn into_event(self) -> anyhow::Result<RenderEvent> {
        Ok(match self {
            FromForkServer::NeedResource { url, kind, deferred } => RenderEvent::NeedResource { url, kind, deferred },
            FromForkServer::Tile(header) => RenderEvent::Tile(header),
            FromForkServer::TileUnchanged(header) => RenderEvent::TileUnchanged(header),
            FromForkServer::PageRendered { summary, hit_regions } => RenderEvent::Rendered {
                summary,
                hit_regions,
                effects: Vec::new(),
            },
            FromForkServer::Refused(reason) => RenderEvent::Refused(reason),
            other => anyhow::bail!("unexpected render-exchange message: {other:?}"),
        })
    }

    fn send_resource(link: &mut Endpoint, reply: ResourceReply) -> std::io::Result<()> {
        link.send(&ToForkServer::Resource(reply))
    }
}

impl RenderStream for FromRenderer {
    fn into_event(self) -> anyhow::Result<RenderEvent> {
        Ok(match self {
            FromRenderer::NeedResource { url, kind, deferred } => RenderEvent::NeedResource { url, kind, deferred },
            FromRenderer::Tile(header) => RenderEvent::Tile(header),
            FromRenderer::TileUnchanged(header) => RenderEvent::TileUnchanged(header),
            FromRenderer::Rendered {
                summary,
                hit_regions,
                effects,
            } => RenderEvent::Rendered {
                summary,
                hit_regions,
                effects,
            },
            FromRenderer::Evict { hashes } => RenderEvent::Evict(hashes),
            FromRenderer::Audit(_) => anyhow::bail!("an audit report in the middle of a render"),
        })
    }

    fn send_resource(link: &mut Endpoint, reply: ResourceReply) -> std::io::Result<()> {
        link.send(&reply)
    }
}

/// Largest body a [`ResourceReply::Ok`] carries in-band: half a frame, leaving the
/// rest of the message room. A larger one goes as [`ResourceReply::Shared`] - in a
/// frame it would fail to send, and a failed send ends the exchange and with it the
/// renderer, which a page with one large image should not be able to do.
const MAX_IN_BAND_RESOURCE: usize = gosub_ipc::MAX_FRAME_LEN as usize / 2;

/// Send `resource` to the renderer as [`ResourceReply::Shared`] and its body as a
/// sealed memfd. A body the link cannot carry - past `gosub_ipc::shm::MAX_BLOB_LEN`,
/// or on a link without fd passing - is answered `Failed`: the renderer goes on
/// without it, as it would without any resource that failed to load.
fn send_shared_resource<M: RenderStream>(link: &mut Endpoint, resource: LoadedResource) -> std::io::Result<()> {
    let len = resource.body.len();
    let fd = if link.tx.supports_fd_passing() {
        gosub_ipc::shm::create_sealed_blob(len, |buf| buf.copy_from_slice(&resource.body))
    } else {
        Err(std::io::Error::other("this link cannot pass fds"))
    };
    match fd {
        Ok(fd) => {
            M::send_resource(
                link,
                ResourceReply::Shared {
                    status: resource.status,
                    content_type: resource.content_type,
                    len: len as u64,
                },
            )?;
            link.tx.send_fd(std::os::fd::AsRawFd::as_raw_fd(&fd))
        }
        Err(e) => M::send_resource(
            link,
            ResourceReply::Failed(format!("a {len}-byte body cannot reach the renderer: {e}")),
        ),
    }
}

/// Drive the broker's half of a render exchange, whoever the renderer is.
/// Tiles stream in one at a time (each fd mapped and released before the
/// next message), the summary closes the exchange, and a `Refused`
/// mid-stream discards everything collected - atomicity lives here, not in
/// transport buffering. `loader` answers the renderer's subresource requests
/// inline, where identity and cookies live.
pub(crate) fn drive_render_exchange<M: RenderStream>(
    link: &mut Endpoint,
    loader: &dyn RenderResources,
    known_tiles: &TileMemory,
) -> anyhow::Result<RenderedPage> {
    let mut received = Vec::new();
    let mut evicted = Vec::new();
    let started = std::time::Instant::now();
    let (mut messages, mut resources, mut tile_bytes) = (0usize, 0usize, 0usize);
    loop {
        messages += 1;
        if messages > MAX_EXCHANGE_MESSAGES {
            anyhow::bail!("renderer sent more than {MAX_EXCHANGE_MESSAGES} messages in one render");
        }
        if started.elapsed() > EXCHANGE_DEADLINE {
            anyhow::bail!("render did not finish within {EXCHANGE_DEADLINE:?}");
        }
        match link.recv::<M>()?.into_event()? {
            RenderEvent::Evict(hashes) => {
                evicted.extend(hashes);
                if evicted.len() > MAX_EXCHANGE_MESSAGES {
                    anyhow::bail!("renderer evicted more than {MAX_EXCHANGE_MESSAGES} tiles in one render");
                }
            }
            RenderEvent::NeedResource { url, kind, deferred } => {
                resources += 1;
                if resources > MAX_EXCHANGE_RESOURCES {
                    anyhow::bail!("renderer asked for more than {MAX_EXCHANGE_RESOURCES} resources in one render");
                }
                let asked = std::time::Instant::now();
                let reply = match url::Url::parse(&url) {
                    Ok(parsed) => {
                        let loaded = if deferred {
                            loader.load_deferred(&parsed, kind)
                        } else {
                            loader.load(&parsed, kind)
                        };
                        if crate::telemetry::enabled() {
                            let (outcome, bytes) = match &loaded {
                                Ok(resource) => ("served", resource.body.len()),
                                Err(LoadError::Pending) => ("pending", 0),
                                Err(_) => ("failed", 0),
                            };
                            crate::telemetry::emit(
                                "remote.resource",
                                serde_json::json!({
                                    "url": url,
                                    "deferred": deferred,
                                    "outcome": outcome,
                                    "bytes": bytes,
                                    "renderer_waited_us": asked.elapsed().as_micros() as u64,
                                }),
                            );
                        }
                        match loaded {
                            Ok(resource) if resource.body.len() > MAX_IN_BAND_RESOURCE => {
                                send_shared_resource::<M>(link, resource)?;
                                continue;
                            }
                            Ok(resource) => ResourceReply::Ok {
                                status: resource.status,
                                content_type: resource.content_type,
                                body: resource.body.to_vec(),
                            },
                            Err(LoadError::Pending) => ResourceReply::Pending,
                            Err(e) => ResourceReply::Failed(e.to_string()),
                        }
                    }
                    Err(e) => ResourceReply::Failed(format!("renderer asked for an unparseable url: {e}")),
                };
                M::send_resource(link, reply)?;
            }
            RenderEvent::Tile(header) => {
                let fd = link.rx.recv_fd()?;
                let mapping = gosub_ipc::shm::map_sealed_tile(fd, header.width, header.height)?;
                tile_bytes += mapping.as_slice().len();
                if tile_bytes > MAX_EXCHANGE_TILE_BYTES {
                    anyhow::bail!("renderer sent more than {MAX_EXCHANGE_TILE_BYTES} bytes of tiles in one render");
                }
                received.push(PageTile::Fresh { header, mapping });
            }
            // The renderer skipped this one because we said we had it. If we
            // do not, our memory and its `known_tiles` disagree - a bug, not
            // a page problem, so fail the render rather than paper over a
            // hole in the page.
            RenderEvent::TileUnchanged(header) => {
                let Some(kept) = known_tiles.get(header.content_hash) else {
                    anyhow::bail!("renderer skipped a tile we do not have (hash {})", header.content_hash);
                };
                received.push(PageTile::Reused { header, kept });
            }
            RenderEvent::Rendered {
                mut summary,
                mut hit_regions,
                mut effects,
            } => {
                bound_summary(&mut summary);
                bound_hit_regions(&mut hit_regions);
                bound_effects(&mut effects)?;
                return Ok(RenderedPage {
                    summary,
                    tiles: received,
                    hit_regions,
                    evicted,
                    effects,
                });
            }
            RenderEvent::Refused(reason) => anyhow::bail!("{reason}"),
        }
    }
}

/// A document message too big for one frame to a renderer is not sent: the
/// send would fail, and a failed send is taken for a renderer gone. Measured
/// whole, since the URL and the tile hashes travel with the document. The
/// caller renders it some other way or reports it.
fn refuse_oversized_document<T: serde::Serialize>(message: &T) -> anyhow::Result<()> {
    let len = gosub_ipc::frame_len(message)?;
    if len > u64::from(gosub_ipc::MAX_FRAME_LEN) {
        anyhow::bail!(
            "a {len}-byte document message is more than a renderer link carries ({})",
            gosub_ipc::MAX_FRAME_LEN
        );
    }
    Ok(())
}

/// Cut a renderer-supplied string to `max` characters. For text that is
/// displayed, never for a URL - see [`drop_long_url`].
fn bound_text(text: &mut String, max: usize) {
    if text.len() > max {
        *text = text.chars().take(max).collect();
    }
}

/// A string that will be shown by the embedder (the window title): control
/// characters and the bidi overrides go, so a page cannot reorder or hide
/// what the embedder displays next to it. Whitespace is kept.
fn displayable(text: &mut String) {
    const BIDI: [char; 9] = [
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    if text
        .chars()
        .any(|c| (c.is_control() && !c.is_whitespace()) || BIDI.contains(&c))
    {
        *text = text
            .chars()
            .filter(|c| !((c.is_control() && !c.is_whitespace()) || BIDI.contains(c)))
            .collect();
    }
}

/// A renderer-supplied URL past [`MAX_HIT_TEXT`] is dropped whole: cut, it
/// would be navigated to or fetched as a different URL.
fn drop_long_url(url: &mut Option<String>) {
    if url.as_ref().is_some_and(|u| u.len() > MAX_HIT_TEXT) {
        *url = None;
    }
}

/// The renderer's summary, sized as this side is willing to keep and forward it.
fn bound_summary(summary: &mut crate::fork_server::protocol::PageSummary) {
    if let Some(title) = summary.title.as_mut() {
        bound_text(title, MAX_TITLE);
        displayable(title);
    }
    drop_long_url(&mut summary.favicon);
    summary.layer_order.truncate(MAX_LAYER_ORDER);
    summary.timings_us.truncate(MAX_TIMINGS);
    // A scroll target is a y the tab will scroll to: finite, or not kept.
    summary.fragment_targets.retain(|t| t.y.is_finite());
    summary
        .fragment_targets
        .truncate(crate::fork_server::protocol::MAX_FRAGMENT_TARGETS);
    for target in summary.fragment_targets.iter_mut() {
        bound_text(&mut target.name, MAX_HIT_TEXT);
    }
    for (name, _) in summary.timings_us.iter_mut() {
        bound_text(name, MAX_TIMING_NAME);
    }
}

/// [`MAX_HIT_REGIONS`](crate::fork_server::protocol::MAX_HIT_REGIONS) and
/// [`MAX_HIT_TEXT_TOTAL`](crate::fork_server::protocol::MAX_HIT_TEXT_TOTAL)
/// are the producer's promises; this is the consumer's.
fn bound_hit_regions(regions: &mut Vec<crate::fork_server::protocol::HitRegion>) {
    regions.truncate(crate::fork_server::protocol::MAX_HIT_REGIONS);
    let mut text_bytes = 0usize;
    for region in regions.iter_mut() {
        drop_long_url(&mut region.link);
        drop_long_url(&mut region.image);
        text_bytes += region.link.as_ref().map_or(0, String::len) + region.image.as_ref().map_or(0, String::len);
        if text_bytes > crate::fork_server::protocol::MAX_HIT_TEXT_TOTAL {
            region.link = None;
            region.image = None;
        }
    }
}

/// What an input pass asks of the broker, as this side is willing to keep:
/// a bounded handful of effects, their strings cut or dropped like a hit
/// region's, their rectangles finite. Past [`MAX_EFFECTS`] the frame is a
/// renderer gone wrong, which ends the exchange like any malformed frame.
fn bound_effects(effects: &mut Vec<Effect>) -> anyhow::Result<()> {
    if effects.len() > MAX_EFFECTS {
        anyhow::bail!(
            "renderer sent {} effects in one pass (limit {MAX_EFFECTS})",
            effects.len()
        );
    }
    let finite = |r: &WireRect| r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite();
    effects.retain(|effect| match effect {
        Effect::Focus { bounds: Some(b), .. } | Effect::Picker { bounds: b, .. } => finite(b),
        _ => true,
    });
    for effect in effects.iter_mut() {
        match effect {
            Effect::Navigate { url, body, .. } => {
                let mut candidate = Some(std::mem::take(url));
                drop_long_url(&mut candidate);
                // A body past the bound is not cut: the whole navigation goes,
                // like a URL past its bound, by emptying what the retain below
                // checks.
                if body.as_ref().is_some_and(|b| b.len() > MAX_FORM_BODY) {
                    candidate = None;
                }
                *url = candidate.unwrap_or_default();
            }
            // The picker's strings reach the embedder's UI. The value is
            // sanitised again for its kind, so it is what the in-process path
            // hands over (`#rrggbb`, an ISO date or time, or empty); the
            // bounds are the attributes as written, made displayable.
            Effect::Picker {
                kind,
                value,
                min,
                max,
                step,
                ..
            } => {
                bound_text(value, MAX_HIT_TEXT);
                *value = crate::engine::edit::sanitize_picker_value(*kind, value);
                for field in [min, max, step].into_iter().flatten() {
                    bound_text(field, MAX_HIT_TEXT);
                    displayable(field);
                }
            }
            Effect::ClipboardWrite { text } => bound_text(text, MAX_CLIPBOARD_TEXT),
            Effect::Focus { .. } | Effect::Cursor { .. } | Effect::PasteRequested | Effect::Capture { .. } => {}
        }
    }
    // A navigation whose URL or body was dropped for its length is no navigation.
    effects.retain(|effect| !matches!(effect, Effect::Navigate { url, .. } if url.is_empty()));
    Ok(())
}

/// One page as the broker receives it: what the renderer measured, its tiles
/// (freshly mapped or reused from the previous render), the geometry hit
/// testing needs, and what an input pass asked for.
#[derive(Debug)]
pub struct RenderedPage {
    pub summary: crate::fork_server::protocol::PageSummary,
    pub tiles: Vec<PageTile>,
    pub hit_regions: Vec<crate::fork_server::protocol::HitRegion>,
    /// Content hashes of tiles the renderer let go of (retained pages only);
    /// the broker drops them from its memory.
    pub evicted: Vec<u64>,
    /// What the input asked of the broker; empty for any other pass.
    pub effects: Vec<Effect>,
}

/// A tile of a rendered page: either pixels that just crossed, or pixels the
/// broker already had and the renderer therefore never produced.
#[derive(Debug)]
pub enum PageTile {
    Fresh {
        header: crate::fork_server::protocol::TileHeader,
        mapping: gosub_ipc::shm::TileMapping,
    },
    Reused {
        header: crate::fork_server::protocol::TileHeader,
        kept: KeptTile,
    },
}

impl PageTile {
    /// This tile's identity and pixels, for a broker keeping it until the
    /// next render (see [`TileMemory::replace_with`]).
    pub fn keep(&self) -> (u64, KeptTile) {
        let (header, kept) = match self {
            PageTile::Fresh { header, mapping } => (
                header,
                KeptTile::from_header(header, bytes::Bytes::copy_from_slice(mapping.as_slice())),
            ),
            PageTile::Reused { header, kept } => (header, kept.clone()),
        };
        (header.content_hash, kept)
    }

    /// This tile's pixels, whichever render produced them. Always the
    /// renderer's own mapped pages - a reused tile is the *same* mapping an
    /// earlier render handed over, not a copy of it.
    pub fn pixels(&self) -> &[u8] {
        match self {
            PageTile::Fresh { mapping, .. } => mapping.as_slice(),
            PageTile::Reused { kept, .. } => kept.pixels.as_ref(),
        }
    }

    /// Hand this tile to the compositor as the [`CachedTile`] the host-side
    /// compositing loop consumes. Zero-copy: a fresh tile's mapping becomes
    /// the `Bytes` owner (`Bytes::from_owner`), so the compositor blends
    /// straight out of the renderer's sealed pages.
    pub fn into_cached_tile(self) -> gosub_interface::render::backend::CachedTile {
        // A reused tile's header is the renderer's `TileUnchanged` placeholder:
        // what the tile looks like, its opacity and anchor included, is what
        // this side kept from the fresh one.
        let (header, width, height, format, pixels, opacity, anchor) = match self {
            PageTile::Fresh { header, mapping } => {
                let (w, h, f, o, a) = (
                    header.width,
                    header.height,
                    header.format,
                    header.opacity,
                    header.anchor,
                );
                (header, w, h, f, bytes::Bytes::from_owner(mapping), o, a)
            }
            PageTile::Reused { header, kept } => (
                header,
                kept.width,
                kept.height,
                kept.format,
                kept.pixels,
                kept.opacity,
                kept.anchor,
            ),
        };
        // Alpha is the 4th byte in both supported formats ([B,G,R,A] / [R,G,B,A]).
        let opaque = pixels.as_chunks::<4>().0.iter().all(|px| px[3] == 0xFF);
        gosub_interface::render::backend::CachedTile {
            page_x: header.page_x as f32,
            page_y: header.page_y as f32,
            width,
            height,
            data: pixels,
            format: format.into(),
            opacity,
            anchor: anchor.into(),
            opaque,
        }
    }
}

/// Pixels the broker keeps between renders of a tab, so an unchanged tile
/// need not be rasterized or shipped again. The bytes are the renderer's
/// mapped pages from an earlier render - still zero-copy, still sealed.
#[derive(Debug, Clone)]
pub struct KeptTile {
    pub page_x: f64,
    pub page_y: f64,
    pub layer_id: u64,
    pub width: u32,
    pub height: u32,
    pub format: crate::fork_server::protocol::TileWireFormat,
    pub opacity: f32,
    pub anchor: crate::fork_server::protocol::TileWireAnchor,
    pub pixels: bytes::Bytes,
}

impl KeptTile {
    /// A fresh tile's header plus its (mapped or copied) pixels.
    pub fn from_header(header: &crate::fork_server::protocol::TileHeader, pixels: bytes::Bytes) -> Self {
        Self {
            page_x: header.page_x,
            page_y: header.page_y,
            layer_id: header.layer_id,
            width: header.width,
            height: header.height,
            format: header.format,
            opacity: header.opacity,
            anchor: header.anchor,
            pixels,
        }
    }

    /// This tile as the compositor's input.
    pub fn to_baked(&self) -> gosub_render_pipeline::rasterizer::BakedTile {
        gosub_render_pipeline::rasterizer::BakedTile {
            page_x: self.page_x,
            page_y: self.page_y,
            layer_id: self.layer_id,
            width: self.width,
            height: self.height,
            format: self.format.into(),
            opacity: self.opacity,
            anchor: self.anchor.into(),
            pixels: gosub_render_pipeline::common::texture::TilePixels::Cpu(self.pixels.clone()),
        }
    }
}

/// Most a tab keeps of its renderer's tiles, in pixel bytes and in tiles. A
/// kept tile pins the renderer's sealed pages in this process, and a pass
/// only takes away what the renderer *says* it evicted - so without this a
/// renderer answering every hover with fresh hashes and no evictions would
/// grow the broker by a pass's worth each time. Generous for an honest page:
/// a 4K viewport's raster window is well under a tenth of it.
pub const MAX_TAB_TILE_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_TAB_TILES: usize = 20_000;

/// What the broker remembers of a tab's last remote render, keyed by content
/// hash - the input to the next render's `known_tiles`.
#[derive(Debug, Default, Clone)]
pub struct TileMemory {
    tiles: std::collections::HashMap<u64, KeptTile>,
    /// Arrival order, oldest first, for what goes when the budget is passed:
    /// (sequence, hash). A removed tile's entry stays and is skipped when it
    /// surfaces, so a pass evicting tens of thousands of hashes costs a map
    /// lookup each, not a scan of this deque each.
    order: std::collections::VecDeque<(u64, u64)>,
    /// The sequence number each kept hash arrived with; a stale deque entry
    /// has another.
    arrived: std::collections::HashMap<u64, u64>,
    next_seq: u64,
    bytes: usize,
}

impl TileMemory {
    /// The hashes to offer a renderer.
    pub fn hashes(&self) -> Vec<u64> {
        self.tiles.keys().copied().collect()
    }

    pub fn get(&self, hash: u64) -> Option<KeptTile> {
        self.tiles.get(&hash).cloned()
    }

    /// Pixel bytes kept.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Replace the memory with exactly this page's tiles: what is not on the
    /// page cannot help the next render of it, and keeping it would grow
    /// without bound.
    pub fn replace_with(&mut self, tiles: impl IntoIterator<Item = (u64, KeptTile)>) {
        self.tiles.clear();
        self.order.clear();
        self.arrived.clear();
        self.bytes = 0;
        self.extend(tiles);
    }

    /// Merge one pass of a retained page: what the renderer let go of leaves,
    /// what it shipped arrives. Past the budget the oldest tiles go too: the
    /// renderer's next `known_tiles` no longer names them, so it ships them
    /// again if the page still needs them.
    pub fn apply_pass(&mut self, evicted: &[u64], tiles: impl IntoIterator<Item = (u64, KeptTile)>) {
        for hash in evicted {
            self.remove(*hash);
        }
        self.extend(tiles);
    }

    fn extend(&mut self, tiles: impl IntoIterator<Item = (u64, KeptTile)>) {
        for (hash, tile) in tiles {
            self.remove(hash);
            self.bytes += tile.pixels.len();
            let seq = self.next_seq;
            self.next_seq += 1;
            self.order.push_back((seq, hash));
            self.arrived.insert(hash, seq);
            self.tiles.insert(hash, tile);
        }
        while self.bytes > MAX_TAB_TILE_BYTES || self.tiles.len() > MAX_TAB_TILES {
            let Some((seq, oldest)) = self.order.pop_front() else {
                break;
            };
            if self.arrived.get(&oldest) != Some(&seq) {
                continue; // removed or re-added since: a stale entry
            }
            self.remove(oldest);
        }
        // Stale entries would otherwise outnumber live ones without bound.
        if self.order.len() > 2 * self.tiles.len() + 64 {
            let arrived = &self.arrived;
            self.order.retain(|(seq, hash)| arrived.get(hash) == Some(seq));
        }
    }

    fn remove(&mut self, hash: u64) {
        if let Some(tile) = self.tiles.remove(&hash) {
            self.bytes -= tile.pixels.len();
            self.arrived.remove(&hash);
        }
    }

    /// Every kept tile as compositor input, back to front: `layer_order` is
    /// the renderer's (a layer it does not name sorts last), then top-down
    /// within a layer - tiles of one layer never overlap, so that order is
    /// only for determinism.
    pub fn baked_tiles(&self, layer_order: &[u64]) -> Vec<gosub_render_pipeline::rasterizer::BakedTile> {
        let ranks: std::collections::HashMap<u64, usize> =
            layer_order.iter().enumerate().map(|(i, &l)| (l, i)).collect();
        let rank = |layer: u64| ranks.get(&layer).copied().unwrap_or(usize::MAX);
        let mut tiles: Vec<&KeptTile> = self.tiles.values().collect();
        tiles.sort_by(|a, b| {
            rank(a.layer_id)
                .cmp(&rank(b.layer_id))
                .then(a.page_y.total_cmp(&b.page_y))
                .then(a.page_x.total_cmp(&b.page_x))
        });
        tiles.into_iter().map(KeptTile::to_baked).collect()
    }
}

/// A running fork server, its announced confinement tier, and the link to it.
pub struct ForkServer {
    link: Endpoint,
    tier: ConfinementTier,
    child: Option<gosub_sandbox::spawn::Child>,
}

impl std::fmt::Debug for ForkServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkServer")
            .field("tier", &self.tier)
            .finish_non_exhaustive()
    }
}

impl ForkServer {
    /// Re-exec this binary as the fork server and wait for its confinement
    /// answer.
    pub fn spawn() -> anyhow::Result<Self> {
        // Same guard as every spawner: an undispatched child must not recurse.
        if crate::child_process::is_child_process() {
            anyhow::bail!(
                "this process was started as an engine child role but is running embedder startup, \
                 which means gosub_engine::child_process::dispatch_with() was not called at the top \
                 of main(); refusing to spawn further processes"
            );
        }

        let exe = std::env::current_exe()?;
        let (ours, theirs) = gosub_ipc::channel::Channel::pair()?;

        let child = gosub_sandbox::spawn::spawn(
            &exe,
            &[crate::child_process::ROLE_FLAG, FORK_SERVER_ROLE],
            theirs,
            // Renderers must not reach the network, and namespace isolation is
            // inherited by everything this process forks.
            gosub_sandbox::NamespaceIsolation::Full,
            gosub_sandbox::spawn::ContainerProfile {
                name: "gosub-fork-server",
                internet: false,
                fs_grant: None,
                // Inherited by every renderer forked from it. A renderer holds
                // a laid-out page, its tiles, and a bounded decoded-image cache,
                // and still needs room for one large image decode on top.
                data_limit: Some(RENDERER_DATA_LIMIT),
                extra_fds: &[],
                // Every resident renderer and its threads live under this one.
                max_tasks: 4096,
                file_size_limit: None,
            },
        )?;
        if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
            log::warn!("could not apply parent-side confinement to the fork server: {e}");
        }

        let mut link = Endpoint::from_channel(ours)?;
        let _ = link.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let _ = link.rx.set_read_timeout(Some(READY_TIMEOUT));

        let tier = match link.recv::<FromForkServer>() {
            Ok(FromForkServer::Ready { tier }) => tier,
            Ok(other) => anyhow::bail!("the fork server sent {other:?} before Ready"),
            Err(e) => anyhow::bail!("the fork server never became ready: {e}"),
        };
        let _ = link.rx.set_read_timeout(Some(REPLY_TIMEOUT));

        Ok(Self {
            link,
            tier,
            child: Some(child),
        })
    }

    /// The confinement tier the configured font system answered - what decides
    /// whether renderer isolation is offered at all, and under which sandbox.
    pub fn confinement(&self) -> &ConfinementTier {
        &self.tier
    }

    /// The fork server's pid, as this process sees it; `None` once shut down.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(|c| c.id())
    }

    /// Fork a renderer, have it shape under its tier sandbox with the
    /// inherited fonts, and return the measured box.
    pub fn prove_shaping(&mut self) -> anyhow::Result<(f32, f32)> {
        self.link.send(&ToForkServer::ForkProof)?;
        match self.link.recv::<FromForkServer>()? {
            FromForkServer::Proof { width, height } => Ok((width, height)),
            FromForkServer::Refused(reason) => anyhow::bail!("{reason}"),
            other => anyhow::bail!("unexpected reply to ForkProof: {other:?}"),
        }
    }

    /// The escape audit, run in the fork server itself.
    pub fn audit(&mut self) -> anyhow::Result<gosub_sandbox::audit::AuditReport> {
        self.audit_exchange(ToForkServer::Audit)
    }

    /// The escape audit, run in a renderer forked for it.
    pub fn audit_forked_renderer(&mut self) -> anyhow::Result<gosub_sandbox::audit::AuditReport> {
        self.audit_exchange(ToForkServer::AuditRenderer)
    }

    /// One audit request, kept to the same rules as a render: a fork server
    /// that is gone is replaced first, and one whose exchange failed is
    /// stopped - a late reply would otherwise answer the next request.
    fn audit_exchange(&mut self, ask: ToForkServer) -> anyhow::Result<gosub_sandbox::audit::AuditReport> {
        self.ensure_running()?;
        let answer = self
            .link
            .send(&ask)
            .map_err(anyhow::Error::from)
            .and_then(|()| self.link.recv::<FromForkServer>().map_err(anyhow::Error::from));
        match answer {
            Ok(FromForkServer::Audit(report)) => Ok(report),
            Ok(FromForkServer::Refused(reason)) => anyhow::bail!("{reason}"),
            Ok(other) => {
                self.stop();
                anyhow::bail!("unexpected reply to an audit: {other:?}")
            }
            Err(e) => {
                self.stop();
                Err(e)
            }
        }
    }

    /// Fork a renderer and run the pipeline over `html` in it - parse, style,
    /// layout, layering, tiling, paint, and (when the configuration has a
    /// forked rasterizer) rasterize - under its tier sandbox, with the
    /// inherited fonts. Returns the measured summary plus the rasterized
    /// tiles, whose pixels arrive as sealed memfds and are mapped - never
    /// copied - into this process.
    #[allow(clippy::too_many_arguments)] // one wire message, spelled out
    pub fn render_page(
        &mut self,
        html: &str,
        url: &str,
        tab: &str,
        viewport: (f64, f64),
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
        hovered_node: Option<u64>,
    ) -> anyhow::Result<RenderedPage> {
        // One that failed an exchange was stopped (see below); a new one takes
        // its place rather than every later render failing with it.
        self.ensure_running()?;
        let rendered = self.exchange_render(html, url, tab, viewport, loader, known_tiles, hovered_node);
        if rendered.is_err() {
            // A failed exchange leaves the link at no known message: the fork
            // server may still be relaying that render's tiles, and the next
            // request would read them as its own reply. Nothing on it can be
            // trusted again.
            self.stop();
        }
        rendered
    }

    #[allow(clippy::too_many_arguments)] // `render_page`'s arguments, passed through
    fn exchange_render(
        &mut self,
        html: &str,
        url: &str,
        tab: &str,
        viewport: (f64, f64),
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
        hovered_node: Option<u64>,
    ) -> anyhow::Result<RenderedPage> {
        let message = ToForkServer::RenderPage {
            html: html.to_string(),
            url: url.to_string(),
            tab: tab.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            dpr: gosub_render_pipeline::render::DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed),
            media: media_prefs(),
            known_tiles: known_tiles.hashes(),
            hovered_node,
        };
        refuse_oversized_document(&message)?;
        self.link.send(&message)?;
        // The render bound for the render, the short one again after: a failed
        // exchange stops this fork server anyway, so only success restores it.
        let _ = self.link.rx.set_read_timeout(Some(RENDER_GAP));
        let rendered = drive_render_exchange::<FromForkServer>(&mut self.link, loader, known_tiles);
        let _ = self.link.rx.set_read_timeout(Some(REPLY_TIMEOUT));
        rendered
    }

    /// Fork a resident renderer and take over its link: from here on the
    /// broker talks to it directly. `label` only names it in `ps`.
    pub fn spawn_renderer(&mut self, label: &str) -> anyhow::Result<ResidentRenderer> {
        // The same recovery as `render_page`: a stopped fork server is
        // replaced, and one whose exchange failed is stopped, since a late
        // `RendererSpawned` or fd left on the link would answer the next
        // request. Resident renderers it forked earlier talk to the broker
        // directly; the pool replaces any that die with it.
        self.ensure_running()?;
        let spawned = self.exchange_spawn(label);
        if spawned.is_err() {
            self.stop();
        }
        spawned
    }

    fn exchange_spawn(&mut self, label: &str) -> anyhow::Result<ResidentRenderer> {
        self.link.send(&ToForkServer::SpawnRenderer {
            label: label.to_string(),
        })?;
        let pid = match self.link.recv::<FromForkServer>()? {
            FromForkServer::RendererSpawned { pid } => pid,
            FromForkServer::Refused(reason) => anyhow::bail!("{reason}"),
            other => anyhow::bail!("unexpected reply to SpawnRenderer: {other:?}"),
        };
        let fd = self.link.rx.recv_fd()?;
        // Claims from a child. The link must be a stream socket before
        // anything is written to it. The pid must be the fork server's own
        // child, by /proc's word: it is placed in its own cgroup (forked, it
        // inherited the fork server's, where one site's renderer could trip a
        // cap shared with every other's) and killed through its pidfd when the
        // broker gives up on it - naming the broker's pid, or any other
        // process's, would have the broker do that to the wrong one. A wrong
        // claim is a hostile fork server, and the caller stops it.
        {
            use std::os::fd::AsRawFd as _;
            if !gosub_ipc::channel::is_stream_socket(fd.as_raw_fd()) {
                anyhow::bail!("the fork server handed over something that is not a stream socket");
            }
        }
        let Some(fork_server) = self.child.as_ref().map(|c| c.id()) else {
            anyhow::bail!("no fork server to have spawned renderer {pid}");
        };
        let pidfd = gosub_sandbox::open_child_pidfd(pid as u32, fork_server)
            .map_err(|e| anyhow::anyhow!("the fork server announced a renderer it did not spawn: {e}"))?;
        if let Err(e) = gosub_sandbox::confine_child_pid(pid as u32, RENDERER_DATA_LIMIT, RENDERER_MAX_TASKS) {
            log::warn!("could not apply parent-side confinement to renderer {pid}: {e}");
        }
        let channel = gosub_ipc::channel::Channel::from_stream(std::os::unix::net::UnixStream::from(fd));
        let mut link = Endpoint::from_channel(channel)?;
        let _ = link.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let _ = link.rx.set_read_timeout(Some(RESIDENT_REPLY_TIMEOUT));
        Ok(ResidentRenderer {
            link,
            pid,
            pidfd: Some(pidfd),
            dead: Default::default(),
        })
    }

    /// Have the fork server collect resident renderers that have exited.
    pub fn reap_exited(&mut self) {
        if self.child.is_none() {
            return;
        }
        let answered = self.link.send(&ToForkServer::ReapExited).is_ok()
            && matches!(self.link.recv::<FromForkServer>(), Ok(FromForkServer::Pong));
        if !answered {
            // Anything but its `Pong` leaves the link out of step.
            self.stop();
        }
    }

    /// Ask for a clean exit, then make sure of it. `&mut self` rather than
    /// consuming, so a handle shared behind a lock (the engine's) can be shut
    /// down in place; afterwards the handle is inert and Drop has nothing to
    /// kill.
    pub fn shutdown(&mut self) {
        let _ = self.link.send(&ToForkServer::Shutdown);
        let Some(mut child) = self.child.take() else {
            return;
        };
        // A fork server stuck in a relay never reads `Shutdown`: give it a
        // moment to leave on its own, then end it, so engine shutdown cannot hang.
        let pid = child.id();
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        while std::time::Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(true) | Err(_)) {
                remove_scratch_dir(FORK_SERVER_ROLE, pid);
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        remove_scratch_dir(FORK_SERVER_ROLE, pid);
    }

    /// A fork server to talk to: this one, unless it was stopped or has
    /// exited (then its link is dead), in which case a fresh one.
    fn ensure_running(&mut self) -> anyhow::Result<()> {
        if self
            .child
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(true) | Err(_)))
        {
            self.stop();
        }
        if self.child.is_none() {
            *self = ForkServer::spawn()?;
        }
        Ok(())
    }

    /// Kill and reap the fork server; the handle is inert until the next render
    /// spawns a replacement.
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            let _ = child.kill();
            let _ = child.wait();
            remove_scratch_dir(FORK_SERVER_ROLE, pid);
        }
    }
}

/// How long [`ForkServer::shutdown`] waits for a clean exit before killing.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// The broker's link to one resident renderer (see `fork_server::resident`):
/// a forked, confined child that outlives its renders. Request/reply is
/// strictly serial, so a handle is used from behind a lock.
pub struct ResidentRenderer {
    link: Endpoint,
    pid: i32,
    /// The process itself, verified at spawn: what the broker kills once it
    /// gives up on the renderer. Asking nicely is for a renderer that still
    /// listens; a hostile one that disarmed its own deadline and spins only
    /// ends this way (`None` in tests that build a handle without a process).
    pidfd: Option<std::os::fd::OwnedFd>,
    /// Set once the link failed: nothing sent afterwards can be trusted to
    /// arrive, and the pool replaces the process on the next request.
    /// Shared and atomic so the pool can read it without taking the lock a
    /// failing exchange may be holding.
    dead: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for ResidentRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentRenderer")
            .field("pid", &self.pid)
            .field("dead", &self.is_dead())
            .finish_non_exhaustive()
    }
}

impl ResidentRenderer {
    /// A handle around a bare link, for tests that play the renderer on the
    /// far end themselves: no process, no pid, nothing to kill.
    #[doc(hidden)]
    pub fn around_link_for_test(link: Endpoint) -> Self {
        Self {
            link,
            pid: 0,
            pidfd: None,
            dead: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// The renderer's pid as this (the broker's) pid namespace numbers it:
    /// `fork` in the fork server returns the number its own namespace sees,
    /// which is the broker's too. Inside the renderers' own namespace the
    /// process has a different, small number (`NSpid` in /proc shows both).
    pub fn pid(&self) -> i32 {
        self.pid
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(std::sync::atomic::Ordering::Acquire)
    }

    /// A handle to the dead flag, readable without this renderer's lock.
    pub fn dead_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.dead)
    }

    /// Dead to the broker is dead: whatever the process is doing - wedged,
    /// spinning, hostile - it ends now, through the pidfd, so a renderer the
    /// broker gave up on never keeps a core or its memory. Idempotent.
    fn mark_dead(&self) {
        if !self.dead.swap(true, std::sync::atomic::Ordering::AcqRel) {
            if let Some(pidfd) = &self.pidfd {
                let _ = gosub_sandbox::pidfd_kill(pidfd);
            }
        }
    }

    fn send(&mut self, msg: &ToRenderer) -> anyhow::Result<()> {
        if self.is_dead() {
            anyhow::bail!("renderer process is gone");
        }
        if let Err(e) = self.link.send(msg) {
            self.mark_dead();
            anyhow::bail!("renderer link failed: {e}");
        }
        Ok(())
    }

    pub fn open_tab(&mut self, tab: &str) -> anyhow::Result<()> {
        self.send(&ToRenderer::OpenTab { tab: tab.to_string() })
    }

    pub fn close_tab(&mut self, tab: &str) -> anyhow::Result<()> {
        self.send(&ToRenderer::CloseTab { tab: tab.to_string() })
    }

    /// Render `html` for `tab` - the raster window around `scroll_y` of it -
    /// and have the renderer retain the page for later [`Self::scroll`]s.
    /// Any failure marks the renderer dead: the exchange strictly alternates,
    /// so a broken one leaves the link in no state a later request could
    /// rely on.
    #[allow(clippy::too_many_arguments)] // one wire message, spelled out
    pub fn navigate(
        &mut self,
        html: &str,
        url: &str,
        tab: &str,
        viewport: (f64, f64),
        scroll_y: f64,
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
        hovered_node: Option<u64>,
    ) -> anyhow::Result<RenderedPage> {
        // Refused before anything is sent: the renderer is still fine.
        let message = ToRenderer::Navigate {
            tab: tab.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            dpr: gosub_render_pipeline::render::DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed),
            media: media_prefs(),
            scroll_y,
            known_tiles: known_tiles.hashes(),
            hovered_node,
        };
        refuse_oversized_document(&message)?;
        self.send(&message)?;
        self.exchange(loader, known_tiles)
    }

    /// The viewport of `tab`'s retained page moved: collect what came into
    /// the raster window (and what the renderer let go of).
    pub fn scroll(
        &mut self,
        tab: &str,
        scroll_y: f64,
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
    ) -> anyhow::Result<RenderedPage> {
        self.send(&ToRenderer::Scroll {
            tab: tab.to_string(),
            scroll_y,
        })?;
        self.exchange(loader, known_tiles)
    }

    /// The user acted on `tab`'s retained page: collect the tiles the input
    /// changed and what it asked of the broker. `known_tiles` is sent along as
    /// on a navigate, since an input that lays the page out again ships it by
    /// content hash.
    pub fn input(
        &mut self,
        tab: &str,
        scroll_y: f64,
        event: InputEvent,
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
    ) -> anyhow::Result<RenderedPage> {
        self.send(&ToRenderer::Input {
            tab: tab.to_string(),
            scroll_y,
            known_tiles: known_tiles.hashes(),
            event,
        })?;
        self.exchange(loader, known_tiles)
    }

    /// The viewport of `tab`'s retained page changed size: the page laid out
    /// again, its window shipped by hash against what the broker holds.
    pub fn resize(
        &mut self,
        tab: &str,
        viewport: (f64, f64),
        scroll_y: f64,
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
    ) -> anyhow::Result<RenderedPage> {
        self.send(&ToRenderer::Resize {
            tab: tab.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            dpr: gosub_render_pipeline::render::DEVICE_PIXEL_RATIO.load(std::sync::atomic::Ordering::Relaxed),
            scroll_y,
            known_tiles: known_tiles.hashes(),
        })?;
        self.exchange(loader, known_tiles)
    }

    /// The pointer moved on `tab`'s retained page: collect the repainted tiles.
    pub fn hover(
        &mut self,
        tab: &str,
        node: Option<u64>,
        loader: &dyn RenderResources,
        known_tiles: &TileMemory,
    ) -> anyhow::Result<RenderedPage> {
        self.send(&ToRenderer::Hover {
            tab: tab.to_string(),
            node,
        })?;
        self.exchange(loader, known_tiles)
    }

    fn exchange(&mut self, loader: &dyn RenderResources, known_tiles: &TileMemory) -> anyhow::Result<RenderedPage> {
        let result = drive_render_exchange::<FromRenderer>(&mut self.link, loader, known_tiles);
        if result.is_err() {
            self.mark_dead();
        }
        result
    }

    /// The escape audit, run inside this resident renderer.
    pub fn audit(&mut self) -> anyhow::Result<gosub_sandbox::audit::AuditReport> {
        self.send(&ToRenderer::Audit)?;
        match self.link.recv::<FromRenderer>() {
            Ok(FromRenderer::Audit(report)) => Ok(report),
            // The link is out of step; nothing read from it later can be trusted.
            Ok(other) => {
                self.mark_dead();
                anyhow::bail!("unexpected reply to Audit: {other:?}")
            }
            Err(e) => {
                self.mark_dead();
                anyhow::bail!("renderer link failed: {e}")
            }
        }
    }

    /// Whether the process is still there, without sending anything: a closed
    /// link reads as end-of-file. Only meaningful between exchanges (the
    /// caller holds the lock, so none is in flight).
    pub fn check_alive(&mut self) -> bool {
        if self.is_dead() {
            return false;
        }
        let alive = self.link.rx.peer_alive();
        if !alive {
            self.mark_dead();
        }
        alive
    }

    /// Make the renderer die mid-life, for tests of what the broker does then.
    pub fn crash_for_test(&mut self) {
        let _ = self.send(&ToRenderer::CrashForTest);
    }

    /// Ask for a clean exit. The process is the fork server's child; it is
    /// reaped there (see [`ForkServer::reap_exited`]).
    pub fn shutdown(&mut self) {
        let _ = self.send(&ToRenderer::Shutdown);
        self.mark_dead();
    }
}

/// The last handle gone is the broker done with the renderer: it is killed,
/// not left to notice end-of-file. The pool lets go of a renderer that is
/// mid-exchange without its lock, and the exchange's thread then drops the
/// last handle; a hostile renderer that stops reading would otherwise live on.
impl Drop for ResidentRenderer {
    fn drop(&mut self) {
        self.mark_dead();
    }
}

impl Drop for ForkServer {
    fn drop(&mut self) {
        // A fork server left running holds warmed page-shaping state for no
        // one; kill-then-reap, the same discipline as the other children.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document bigger than a frame is refused before it is sent, and the
    /// renderer is not marked dead for it.
    #[test]
    fn an_oversized_document_leaves_the_renderer_alive() {
        let (ours, _theirs) = gosub_ipc::channel::Channel::pair().expect("link pair");
        let mut renderer = ResidentRenderer::around_link_for_test(Endpoint::from_channel(ours).expect("endpoint"));
        struct Nothing;
        impl RenderResources for Nothing {
            fn load(&self, _: &url::Url, _: ResourceKind) -> Result<LoadedResource, LoadError> {
                Err(LoadError::Pending)
            }
        }
        // Past the frame cap itself: the size whose send used to fail and
        // take the renderer with it. Then a document that fits on its own,
        // with a URL that takes the message past the cap.
        let cap = gosub_ipc::MAX_FRAME_LEN as usize;
        let long_url = format!("https://site.test/{}", "u".repeat(128 * 1024));
        let cases = [
            ("x".repeat(cap + 1), "https://site.test/".to_string()),
            ("x".repeat(cap - 64 * 1024), long_url),
        ];
        for (html, url) in &cases {
            let result = renderer.navigate(
                html,
                url,
                "tab",
                (800.0, 600.0),
                0.0,
                &Nothing,
                &TileMemory::default(),
                None,
            );
            assert!(result.is_err());
            assert!(!renderer.is_dead(), "refused, not crashed");
        }
    }

    /// A renderer whose last handle is dropped is killed, whatever it is doing.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_dropped_renderer_is_killed() {
        let mut child = std::process::Command::new("sleep").arg("60").spawn().expect("sleep");
        let pidfd = gosub_sandbox::open_child_pidfd(child.id(), std::process::id()).expect("pidfd");
        let (ours, _theirs) = gosub_ipc::channel::Channel::pair().expect("link pair");
        let mut renderer = ResidentRenderer::around_link_for_test(Endpoint::from_channel(ours).expect("endpoint"));
        renderer.pid = child.id() as i32;
        renderer.pidfd = Some(pidfd);
        drop(renderer);
        let started = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("wait") {
                break status;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "still running after the drop"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "SIGKILL");
    }

    /// A body too large for one frame reaches the renderer whole, through a sealed
    /// memfd, where it used to fail the send and with it the renderer; one past
    /// what a blob may hold is answered `Failed` and the exchange goes on.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_body_too_large_for_a_frame_reaches_the_renderer() {
        use crate::fork_server::loader::ForkedResourceLoader;
        use crate::net::resource_loader::ResourceLoader;

        struct Bodies;
        impl RenderResources for Bodies {
            fn load(&self, url: &url::Url, _: ResourceKind) -> Result<LoadedResource, LoadError> {
                let len = match url.path() {
                    "/in-band" => MAX_IN_BAND_RESOURCE,
                    "/shared" => MAX_IN_BAND_RESOURCE + 1,
                    _ => gosub_ipc::shm::MAX_BLOB_LEN + 1,
                };
                Ok(LoadedResource {
                    status: 200,
                    content_type: Some("image/png".into()),
                    body: bytes::Bytes::from((0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>()),
                })
            }
        }

        let (ours, theirs) = gosub_ipc::channel::Channel::pair().expect("link pair");
        let broker = std::thread::spawn(move || {
            let mut link = Endpoint::from_channel(ours).expect("endpoint");
            // Ends with an error once the renderer side hangs up; the loads are the point.
            let _ = drive_render_exchange::<FromRenderer>(&mut link, &Bodies, &TileMemory::default());
        });

        let loader = ForkedResourceLoader::disconnected();
        loader.connect(std::sync::Arc::new(parking_lot::Mutex::new(
            Endpoint::from_channel(theirs).expect("endpoint"),
        )));
        let url = |path: &str| url::Url::parse(&format!("https://site.test{path}")).unwrap();
        let expected = |len: usize| (0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>();

        let in_band = ResourceLoader::load(&*loader, &url("/in-band"), ResourceKind::Image).expect("in band");
        assert_eq!(in_band.body.len(), MAX_IN_BAND_RESOURCE);
        let shared = ResourceLoader::load(&*loader, &url("/shared"), ResourceKind::Image).expect("through a memfd");
        assert_eq!(shared.status, 200);
        assert_eq!(shared.content_type.as_deref(), Some("image/png"));
        assert_eq!(&shared.body[..], &expected(MAX_IN_BAND_RESOURCE + 1)[..]);
        match ResourceLoader::load(&*loader, &url("/too-large"), ResourceKind::Image) {
            Err(LoadError::Failed(reason)) => assert!(reason.contains("cannot reach the renderer"), "{reason}"),
            other => panic!("expected a failed load, got {other:?}"),
        }
        // Still talking after all three: the exchange survived.
        assert!(ResourceLoader::load(&*loader, &url("/in-band"), ResourceKind::Image).is_ok());

        drop(loader);
        broker.join().expect("broker thread");
    }

    /// What an input pass may ask for is bounded like a hit region: over the cap
    /// is a crash, a long URL or an oversized body drops the navigation whole,
    /// a non-finite rectangle drops its effect, display strings are cut.
    #[test]
    fn effects_are_bounded_before_the_broker_sees_them() {
        use crate::fork_server::protocol::{Effect, HitCursor, WireRect, MAX_EFFECTS};

        let mut too_many: Vec<Effect> = (0..=MAX_EFFECTS).map(|_| Effect::PasteRequested).collect();
        assert!(
            bound_effects(&mut too_many).is_err(),
            "over the cap is a renderer gone wrong"
        );

        let rect = WireRect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let mut effects = vec![
            Effect::Navigate {
                url: "https://a.test/ok".into(),
                post: true,
                body: Some("a=1".into()),
            },
            Effect::Navigate {
                url: format!("https://a.test/{}", "x".repeat(MAX_HIT_TEXT)),
                post: false,
                body: None,
            },
            Effect::Navigate {
                url: "https://a.test/big".into(),
                post: true,
                body: Some("b".repeat(MAX_FORM_BODY + 1)),
            },
            Effect::Focus {
                focused: true,
                editable: true,
                bounds: Some(WireRect { x: f64::NAN, ..rect }),
            },
            Effect::Picker {
                kind: crate::engine::events::PickerKind::Date,
                bounds: rect,
                value: "2026-10-08".into(),
                min: Some("9".repeat(MAX_HIT_TEXT + 5)),
                max: None,
                step: None,
            },
            Effect::Cursor {
                cursor: HitCursor::Text,
            },
        ];
        bound_effects(&mut effects).expect("a bounded list passes");
        let kept: Vec<String> = effects
            .iter()
            .map(|e| match e {
                Effect::Navigate { url, body, .. } => {
                    format!("nav {url} body={}", body.as_ref().map_or(0, String::len))
                }
                Effect::Focus { .. } => "focus".into(),
                Effect::Picker { min, .. } => format!("picker {}", min.as_ref().map_or(0, String::len)),
                Effect::Cursor { .. } => "cursor".into(),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kept,
            vec![
                "nav https://a.test/ok body=3".to_string(),
                format!("picker {MAX_HIT_TEXT}"),
                "cursor".to_string(),
            ],
            "the long URL, the oversized body and the NaN focus rectangle are gone; the rest is cut, not dropped"
        );
    }
    use crate::fork_server::protocol::{HitCursor, TileHeader, TileWireAnchor, TileWireFormat, MAX_HIT_TEXT_TOTAL};

    fn region(link: Option<String>) -> HitRegion {
        HitRegion {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            node_id: 1,
            anchor: TileWireAnchor::Scroll,
            link,
            image: None,
            cursor: HitCursor::Pointer,
            editable: false,
        }
    }

    fn kept(bytes: usize) -> KeptTile {
        KeptTile {
            page_x: 0.0,
            page_y: 0.0,
            layer_id: 1,
            width: 1,
            height: 1,
            format: TileWireFormat::Rgba8,
            opacity: 1.0,
            anchor: TileWireAnchor::Scroll,
            pixels: bytes::Bytes::from(vec![0u8; bytes]),
        }
    }

    /// A pass that evicts nothing cannot grow a tab past its budget: the
    /// oldest tiles go, and the renderer is simply not told it has them.
    #[test]
    fn tile_memory_is_bounded_across_passes() {
        let tile = MAX_TAB_TILE_BYTES / 4;
        let mut memory = TileMemory::default();
        memory.replace_with((0..4u64).map(|h| (h, kept(tile))));
        assert_eq!(memory.bytes(), MAX_TAB_TILE_BYTES);
        // Two more passes, each shipping fresh hashes and evicting nothing.
        memory.apply_pass(&[], [(10, kept(tile))]);
        memory.apply_pass(&[], [(11, kept(tile))]);
        assert!(memory.bytes() <= MAX_TAB_TILE_BYTES);
        assert!(
            memory.get(0).is_none() && memory.get(1).is_none(),
            "the oldest went first"
        );
        assert!(
            memory.get(10).is_some() && memory.get(11).is_some(),
            "the newest stayed"
        );
        // What the renderer evicts is removed and its bytes given back.
        memory.apply_pass(&[10, 11], []);
        assert_eq!(memory.bytes(), 2 * tile);
        assert_eq!(memory.hashes().len(), 2);
    }

    /// A picker's strings reach the embedder's UI: the value is what the
    /// in-process path would hand over for its kind, the bounds lose control
    /// and bidi characters.
    #[test]
    fn picker_strings_are_displayable() {
        use crate::engine::events::PickerKind;
        use crate::fork_server::protocol::{Effect, WireRect};

        let picker = |kind, value: &str| Effect::Picker {
            kind,
            bounds: WireRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            value: value.into(),
            min: Some("2026\u{202E}-01-01\x1b[0m".into()),
            max: None,
            step: Some("1\n".into()),
        };
        let mut effects = vec![
            picker(PickerKind::Date, "Pay \u{202E}evil\u{202C} bank"),
            picker(PickerKind::Date, " 2026-10-08 "),
            picker(PickerKind::Color, "rebeccapurple"),
            picker(PickerKind::Color, "\x1b[2Jnot a colour"),
        ];
        bound_effects(&mut effects).expect("four effects pass");
        let seen: Vec<_> = effects
            .iter()
            .map(|e| match e {
                Effect::Picker {
                    value, min, max, step, ..
                } => (value.as_str(), min.as_deref(), max.as_deref(), step.as_deref()),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        let bounds = (Some("2026-01-01[0m"), None, Some("1\n"));
        assert_eq!(
            seen,
            vec![
                ("", bounds.0, bounds.1, bounds.2),
                ("2026-10-08", bounds.0, bounds.1, bounds.2),
                ("#663399", bounds.0, bounds.1, bounds.2),
                ("#000000", bounds.0, bounds.1, bounds.2),
            ]
        );
    }

    /// A title reaches the embedder's window: no control or bidi characters.
    #[test]
    fn a_title_is_displayable() {
        let mut summary = crate::fork_server::protocol::PageSummary {
            title: Some("Pay \u{202E}evil\u{202C} bank\x1b[0m\u{7f} ok\n".into()),
            ..Default::default()
        };
        bound_summary(&mut summary);
        assert_eq!(summary.title.as_deref(), Some("Pay evil bank[0m ok\n"));
    }

    /// Thousands of evictions in one pass cost a lookup each, and the
    /// arrival order still decides what goes past the budget.
    #[test]
    fn tile_memory_evictions_are_cheap_and_order_survives() {
        let mut memory = TileMemory::default();
        memory.replace_with((0..20_000u64).map(|h| (h, kept(16))));
        let evicted: Vec<u64> = (0..19_990).collect();
        let started = std::time::Instant::now();
        memory.apply_pass(&evicted, [(50_000, kept(16))]);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "evictions scanned the deque"
        );
        assert_eq!(memory.hashes().len(), 11);
        // Past the count budget, what arrived first goes first: the survivors
        // 19_990..19_999, then 50_000.
        memory.apply_pass(&[], (60_000..60_000 + MAX_TAB_TILES as u64).map(|h| (h, kept(16))));
        assert!(memory.get(19_990).is_none() && memory.get(50_000).is_none());
        assert!(memory.get(60_000 + MAX_TAB_TILES as u64 - 1).is_some());
        assert_eq!(memory.hashes().len(), MAX_TAB_TILES);
    }

    /// A URL past the bound is dropped whole: cut, it would be navigated to.
    #[test]
    fn a_link_past_the_bound_is_dropped_not_cut() {
        let long = format!("https://example.test/?{}", "x".repeat(MAX_HIT_TEXT));
        let mut regions = vec![region(Some(long)), region(Some("https://example.test/ok".into()))];
        bound_hit_regions(&mut regions);
        assert_eq!(regions[0].link, None);
        assert_eq!(
            regions[0].cursor,
            HitCursor::Pointer,
            "the box still hit-tests as a link"
        );
        assert_eq!(regions[1].link.as_deref(), Some("https://example.test/ok"));
    }

    /// Past the page's link-text budget the regions stay, their strings go.
    #[test]
    fn link_text_past_the_page_budget_is_dropped() {
        let each = format!("https://example.test/{}", "y".repeat(1000));
        let count = MAX_HIT_TEXT_TOTAL / each.len() + 2;
        let mut regions: Vec<HitRegion> = (0..count).map(|_| region(Some(each.clone()))).collect();
        bound_hit_regions(&mut regions);
        assert_eq!(regions.len(), count, "no region was dropped");
        assert!(regions.first().unwrap().link.is_some(), "the budget covers the first");
        assert!(regions.last().unwrap().link.is_none(), "the last is past the budget");
        let kept: usize = regions.iter().filter_map(|r| r.link.as_ref()).map(String::len).sum();
        assert!(kept <= MAX_HIT_TEXT_TOTAL);
    }

    fn loaded(len: usize) -> Result<LoadedResource, String> {
        Ok(LoadedResource {
            status: 200,
            content_type: None,
            body: bytes::Bytes::from(vec![0u8; len]),
        })
    }

    /// An image bigger than the whole budget is a failed load, kept as one,
    /// not evicted as it lands and fetched again on every render.
    #[test]
    fn an_image_past_the_budget_is_kept_as_a_failure() {
        let mut entries = MediaEntries::default();
        entries.insert("big".into(), loaded(MEDIA_CACHE_BUDGET + 1));
        assert!(matches!(entries.by_url.get("big"), Some(Err(_))));
        assert_eq!(entries.bytes, 0);
    }

    /// Past the budget, older entries make room for the one just stored.
    #[test]
    fn older_entries_make_room_for_a_new_one() {
        let mut entries = MediaEntries::default();
        let half = MEDIA_CACHE_BUDGET / 2 + 1;
        entries.insert("old".into(), loaded(half));
        entries.insert("new".into(), loaded(half));
        assert!(entries.by_url.contains_key("new"));
        assert!(!entries.by_url.contains_key("old"));
        assert_eq!(entries.bytes, half);
    }

    /// Storing a key again replaces it: one place in the order, its bytes once.
    #[test]
    fn a_key_stored_twice_is_counted_once() {
        let mut entries = MediaEntries::default();
        entries.insert("a".into(), loaded(10));
        entries.insert("a".into(), loaded(10));
        assert_eq!(entries.bytes, 10);
        assert_eq!(entries.order.len(), 1);
    }

    /// Failed and empty loads count as entries: the byte budget never lets
    /// them go, the entry cap does.
    #[test]
    fn entries_are_capped_failures_included() {
        let mut entries = MediaEntries::default();
        for i in 0..MAX_MEDIA_ENTRIES + 10 {
            entries.insert(format!("u{i}"), Err("404".into()));
        }
        assert_eq!(entries.by_url.len(), MAX_MEDIA_ENTRIES);
        assert_eq!(entries.order.len(), MAX_MEDIA_ENTRIES);
        assert!(
            entries.by_url.contains_key(&format!("u{}", MAX_MEDIA_ENTRIES + 9)),
            "the newest stays"
        );
        assert!(!entries.by_url.contains_key("u0"), "the oldest goes");
    }

    /// A full fetch queue answers a further image as failed, without queueing
    /// it or leaving it marked in flight.
    #[test]
    fn a_full_queue_fails_the_next_image() {
        let cache = std::sync::Arc::new(RemoteMediaCache::default());
        {
            let mut queue = cache.queue.lock();
            queue.fetchers = MAX_MEDIA_FETCHERS;
            let url = url::Url::parse("https://img.test/waiting").unwrap();
            let loader: MediaLoader = std::sync::Arc::new(crate::net::resource_loader::NoResourceLoader);
            for _ in 0..MAX_MEDIA_QUEUE {
                queue
                    .waiting
                    .push_back((url.clone(), ResourceKind::Image, std::sync::Arc::clone(&loader)));
            }
        }
        let url = url::Url::parse("https://img.test/one-more").unwrap();
        let loader: MediaLoader = std::sync::Arc::new(crate::net::resource_loader::NoResourceLoader);
        let answer = cache.lookup_or_fetch(&url, ResourceKind::Image, loader);
        assert!(matches!(answer, Err(LoadError::Failed(_))), "{answer:?}");
        assert_eq!(cache.queue.lock().waiting.len(), MAX_MEDIA_QUEUE);
        assert!(!cache.in_flight.lock().contains(url.as_str()));
    }

    /// A reused tile keeps the opacity and anchor it was shipped with; the
    /// `TileUnchanged` header carries only placeholders for them.
    #[test]
    fn a_reused_tile_keeps_its_opacity_and_anchor() {
        let placeholder = TileHeader {
            page_x: 0.0,
            page_y: 0.0,
            layer_id: 7,
            width: 1,
            height: 1,
            format: TileWireFormat::Rgba8,
            content_hash: 42,
            opacity: 1.0,
            anchor: TileWireAnchor::Scroll,
        };
        let kept = KeptTile {
            page_x: 0.0,
            page_y: 0.0,
            layer_id: 7,
            width: 1,
            height: 1,
            format: TileWireFormat::Rgba8,
            opacity: 0.5,
            anchor: TileWireAnchor::Fixed,
            pixels: bytes::Bytes::from_static(&[0, 0, 0, 0xFF]),
        };
        let cached = PageTile::Reused {
            header: placeholder,
            kept,
        }
        .into_cached_tile();
        assert_eq!(cached.opacity, 0.5);
        assert!(
            matches!(cached.anchor, gosub_interface::render::backend::TileAnchor::Fixed),
            "{:?}",
            cached.anchor
        );
    }
}
