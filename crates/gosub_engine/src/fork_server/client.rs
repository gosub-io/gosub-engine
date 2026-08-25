//! The broker's side of the fork server: spawn it, learn its confinement
//! tier, ask it to fork.

use crate::fork_server::protocol::{
    ConfinementTier, FromForkServer, FromRenderer, HitRegion, PageSummary, ResourceReply, TileHeader, ToForkServer,
    ToRenderer,
};
use gosub_interface::resource_loader::{LoadError, LoadedResource};
use gosub_ipc::Endpoint;
use std::time::Duration;

/// The argv role name the broker re-execs itself with.
pub const FORK_SERVER_ROLE: &str = "fork-server";

/// How long to wait for `Ready`. Spawn plus font warm-up: the slowest measured
/// preparation (full warm-up on a font-heavy host) is well under a second, so
/// tens of seconds means a process that is not a fork server.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// How long any later request may take. A fork plus one shape is milliseconds.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// What answers a renderer's subresource requests during an exchange: the
/// broker's loader, plus - for a tab - a cache that lets an image request be
/// answered at once and fetched in the background.
pub trait RenderResources {
    fn load(&self, url: &url::Url) -> Result<LoadedResource, LoadError>;
    /// A resource the render can do without for now (an image). The default
    /// fetches it anyway - correct, just not asynchronous.
    fn load_deferred(&self, url: &url::Url) -> Result<LoadedResource, LoadError> {
        self.load(url)
    }
}

impl<T: gosub_interface::resource_loader::ResourceLoader + ?Sized> RenderResources for T {
    fn load(&self, url: &url::Url) -> Result<LoadedResource, LoadError> {
        gosub_interface::resource_loader::ResourceLoader::load(self, url)
    }
}

/// Subresources fetched on a tab's behalf, images asynchronously.
pub struct TabResources {
    pub loader: std::sync::Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    pub media: std::sync::Arc<RemoteMediaCache>,
}

impl RenderResources for TabResources {
    fn load(&self, url: &url::Url) -> Result<LoadedResource, LoadError> {
        self.loader.load(url)
    }

    fn load_deferred(&self, url: &url::Url) -> Result<LoadedResource, LoadError> {
        self.media.lookup_or_fetch(url, std::sync::Arc::clone(&self.loader))
    }
}

/// Images a renderer asked for on a tab's behalf: what has arrived, and what
/// is still on its way. A miss starts the fetch on a thread and answers
/// [`LoadError::Pending`]; when the bytes land, `completed` rises and the tab
/// renders again, this time finding them here.
#[derive(Default)]
pub struct RemoteMediaCache {
    entries: parking_lot::Mutex<std::collections::HashMap<String, Result<LoadedResource, String>>>,
    in_flight: parking_lot::Mutex<std::collections::HashSet<String>>,
    completed: std::sync::atomic::AtomicBool,
}

impl RemoteMediaCache {
    pub fn lookup_or_fetch(
        self: &std::sync::Arc<Self>,
        url: &url::Url,
        loader: std::sync::Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    ) -> Result<LoadedResource, LoadError> {
        let key = url.to_string();
        if let Some(entry) = self.entries.lock().get(&key) {
            return entry.clone().map_err(LoadError::Failed);
        }
        if self.in_flight.lock().insert(key.clone()) {
            let cache = std::sync::Arc::clone(self);
            let url = url.clone();
            let spawned = std::thread::Builder::new()
                .name("gosub-remote-media".into())
                .spawn(move || {
                    let fetched = loader.load(&url).map_err(|e| e.to_string());
                    cache.entries.lock().insert(url.to_string(), fetched);
                    cache.in_flight.lock().remove(url.as_str());
                    cache.completed.store(true, std::sync::atomic::Ordering::Release);
                });
            if spawned.is_err() {
                self.in_flight.lock().remove(&key);
                return Err(LoadError::Failed("could not start the image fetch".into()));
            }
        }
        Err(LoadError::Pending)
    }

    /// Whether an image landed since the last call - the tab should render
    /// again to pick it up.
    pub fn take_completed(&self) -> bool {
        self.completed.swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Forget everything (a new document).
    pub fn clear(&self) {
        self.entries.lock().clear();
        self.in_flight.lock().clear();
        self.completed.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// One step of a render exchange, whichever peer is speaking.
pub(crate) enum RenderEvent {
    NeedResource {
        url: String,
        deferred: bool,
    },
    Tile(TileHeader),
    TileUnchanged(TileHeader),
    Rendered {
        summary: PageSummary,
        hit_regions: Vec<HitRegion>,
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
            FromForkServer::NeedResource { url, deferred } => RenderEvent::NeedResource { url, deferred },
            FromForkServer::Tile(header) => RenderEvent::Tile(header),
            FromForkServer::TileUnchanged(header) => RenderEvent::TileUnchanged(header),
            FromForkServer::PageRendered { summary, hit_regions } => RenderEvent::Rendered { summary, hit_regions },
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
            FromRenderer::NeedResource { url, deferred } => RenderEvent::NeedResource { url, deferred },
            FromRenderer::Tile(header) => RenderEvent::Tile(header),
            FromRenderer::TileUnchanged(header) => RenderEvent::TileUnchanged(header),
            FromRenderer::Rendered { summary, hit_regions } => RenderEvent::Rendered { summary, hit_regions },
            FromRenderer::Evict { hashes } => RenderEvent::Evict(hashes),
        })
    }

    fn send_resource(link: &mut Endpoint, reply: ResourceReply) -> std::io::Result<()> {
        link.send(&reply)
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
    loop {
        match link.recv::<M>()?.into_event()? {
            RenderEvent::Evict(hashes) => evicted.extend(hashes),
            RenderEvent::NeedResource { url, deferred } => {
                let asked = std::time::Instant::now();
                let reply = match url::Url::parse(&url) {
                    Ok(parsed) => {
                        let loaded = if deferred {
                            loader.load_deferred(&parsed)
                        } else {
                            loader.load(&parsed)
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
            RenderEvent::Rendered { summary, hit_regions } => {
                return Ok(RenderedPage {
                    summary,
                    tiles: received,
                    hit_regions,
                    evicted,
                })
            }
            RenderEvent::Refused(reason) => anyhow::bail!("{reason}"),
        }
    }
}

/// One page as the broker receives it: what the renderer measured, its tiles
/// (freshly mapped or reused from the previous render), and the geometry hit
/// testing needs.
#[derive(Debug)]
pub struct RenderedPage {
    pub summary: crate::fork_server::protocol::PageSummary,
    pub tiles: Vec<PageTile>,
    pub hit_regions: Vec<crate::fork_server::protocol::HitRegion>,
    /// Content hashes of tiles the renderer let go of (retained pages only);
    /// the broker drops them from its memory.
    pub evicted: Vec<u64>,
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
        let (header, width, height, format, pixels) = match self {
            PageTile::Fresh { header, mapping } => {
                let (w, h, f) = (header.width, header.height, header.format);
                (header, w, h, f, bytes::Bytes::from_owner(mapping))
            }
            PageTile::Reused { header, kept } => (header, kept.width, kept.height, kept.format, kept.pixels),
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
            opacity: header.opacity,
            anchor: header.anchor.into(),
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

/// What the broker remembers of a tab's last remote render, keyed by content
/// hash - the input to the next render's `known_tiles`.
#[derive(Debug, Default)]
pub struct TileMemory {
    tiles: std::collections::HashMap<u64, KeptTile>,
}

impl TileMemory {
    /// The hashes to offer a renderer.
    pub fn hashes(&self) -> Vec<u64> {
        self.tiles.keys().copied().collect()
    }

    pub fn get(&self, hash: u64) -> Option<KeptTile> {
        self.tiles.get(&hash).cloned()
    }

    /// Replace the memory with exactly this page's tiles: what is not on the
    /// page cannot help the next render of it, and keeping it would grow
    /// without bound.
    pub fn replace_with(&mut self, tiles: impl IntoIterator<Item = (u64, KeptTile)>) {
        self.tiles = tiles.into_iter().collect();
    }

    /// Merge one pass of a retained page: what the renderer let go of leaves,
    /// what it shipped arrives.
    pub fn apply_pass(&mut self, evicted: &[u64], tiles: impl IntoIterator<Item = (u64, KeptTile)>) {
        for hash in evicted {
            self.tiles.remove(hash);
        }
        self.tiles.extend(tiles);
    }

    /// Every kept tile as compositor input, back to front: `layer_order` is
    /// the renderer's (a layer it does not name sorts last), then top-down
    /// within a layer - tiles of one layer never overlap, so that order is
    /// only for determinism.
    pub fn baked_tiles(&self, layer_order: &[u64]) -> Vec<gosub_render_pipeline::rasterizer::BakedTile> {
        let rank = |layer: u64| layer_order.iter().position(|&l| l == layer).unwrap_or(usize::MAX);
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
        self.link.send(&ToForkServer::RenderPage {
            html: html.to_string(),
            url: url.to_string(),
            tab: tab.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            known_tiles: known_tiles.hashes(),
            hovered_node,
        })?;
        drive_render_exchange::<FromForkServer>(&mut self.link, loader, known_tiles)
    }

    /// Fork a resident renderer and take over its link: from here on the
    /// broker talks to it directly. `label` only names it in `ps`.
    pub fn spawn_renderer(&mut self, label: &str) -> anyhow::Result<ResidentRenderer> {
        self.link.send(&ToForkServer::SpawnRenderer {
            label: label.to_string(),
        })?;
        let pid = match self.link.recv::<FromForkServer>()? {
            FromForkServer::RendererSpawned { pid } => pid,
            FromForkServer::Refused(reason) => anyhow::bail!("{reason}"),
            other => anyhow::bail!("unexpected reply to SpawnRenderer: {other:?}"),
        };
        let fd = self.link.rx.recv_fd()?;
        let channel = gosub_ipc::channel::Channel::from_stream(std::os::unix::net::UnixStream::from(fd));
        let mut link = Endpoint::from_channel(channel)?;
        let _ = link.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let _ = link.rx.set_read_timeout(Some(REPLY_TIMEOUT));
        Ok(ResidentRenderer { link, pid, dead: false })
    }

    /// Have the fork server collect resident renderers that have exited.
    pub fn reap_exited(&mut self) {
        if self.link.send(&ToForkServer::ReapExited).is_ok() {
            let _ = self.link.recv::<FromForkServer>();
        }
    }

    /// Ask for a clean exit, then make sure of it. `&mut self` rather than
    /// consuming, so a handle shared behind a lock (the engine's) can be shut
    /// down in place; afterwards the handle is inert and Drop has nothing to
    /// kill.
    pub fn shutdown(&mut self) {
        let _ = self.link.send(&ToForkServer::Shutdown);
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

/// The broker's link to one resident renderer (see `fork_server::resident`):
/// a forked, confined child that outlives its renders. Request/reply is
/// strictly serial, so a handle is used from behind a lock.
pub struct ResidentRenderer {
    link: Endpoint,
    pid: i32,
    /// Set once the link failed: nothing sent afterwards can be trusted to
    /// arrive, and the pool replaces the process on the next request.
    dead: bool,
}

impl std::fmt::Debug for ResidentRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentRenderer")
            .field("pid", &self.pid)
            .field("dead", &self.dead)
            .finish_non_exhaustive()
    }
}

impl ResidentRenderer {
    /// The renderer's pid as this (the broker's) pid namespace numbers it:
    /// `fork` in the fork server returns the number its own namespace sees,
    /// which is the broker's too. Inside the renderers' own namespace the
    /// process has a different, small number (`NSpid` in /proc shows both).
    pub fn pid(&self) -> i32 {
        self.pid
    }

    pub fn is_dead(&self) -> bool {
        self.dead
    }

    fn send(&mut self, msg: &ToRenderer) -> anyhow::Result<()> {
        if self.dead {
            anyhow::bail!("renderer process is gone");
        }
        if let Err(e) = self.link.send(msg) {
            self.dead = true;
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
        self.send(&ToRenderer::Navigate {
            tab: tab.to_string(),
            html: html.to_string(),
            url: url.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            scroll_y,
            known_tiles: known_tiles.hashes(),
            hovered_node,
        })?;
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
            self.dead = true;
        }
        result
    }

    /// Whether the process is still there, without sending anything: a closed
    /// link reads as end-of-file. Only meaningful between exchanges (the
    /// caller holds the lock, so none is in flight).
    pub fn check_alive(&mut self) -> bool {
        if self.dead {
            return false;
        }
        let alive = self.link.rx.peer_alive();
        if !alive {
            self.dead = true;
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
        self.dead = true;
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
