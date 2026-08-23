//! The broker's side of the fork server: spawn it, learn its confinement
//! tier, ask it to fork.

use crate::fork_server::protocol::{ConfinementTier, FromForkServer, ToForkServer};
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

/// Drive the broker's half of a render exchange, whoever the renderer is -
/// the fork server's forked child or a fresh exec'd renderer speak the same
/// dialect. Tiles stream in one at a time (each fd mapped and released before
/// the next message), the summary closes the exchange, and a `Refused`
/// mid-stream discards everything collected - atomicity lives here, not in
/// transport buffering. `loader` answers the renderer's subresource requests
/// inline, where identity and cookies live.
pub(crate) fn drive_render_exchange(
    link: &mut Endpoint,
    loader: &dyn gosub_interface::resource_loader::ResourceLoader,
    known_tiles: &TileMemory,
) -> anyhow::Result<RenderedPage> {
    use crate::fork_server::protocol::ResourceReply;

    let mut received = Vec::new();
    loop {
        match link.recv::<FromForkServer>()? {
            FromForkServer::NeedResource { url } => {
                let reply = match url::Url::parse(&url) {
                    Ok(parsed) => match loader.load(&parsed) {
                        Ok(resource) => ResourceReply::Ok {
                            status: resource.status,
                            content_type: resource.content_type,
                            body: resource.body.to_vec(),
                        },
                        Err(e) => ResourceReply::Failed(e.to_string()),
                    },
                    Err(e) => ResourceReply::Failed(format!("renderer asked for an unparseable url: {e}")),
                };
                link.send(&ToForkServer::Resource(reply))?;
            }
            FromForkServer::Tile(header) => {
                let fd = link.rx.recv_fd()?;
                let mapping = gosub_ipc::shm::map_sealed_tile(fd, header.width, header.height)?;
                received.push(PageTile::Fresh { header, mapping });
            }
            // The renderer skipped this one because we said we had it. If we
            // do not, our memory and its `known_tiles` disagree - a bug, not
            // a page problem, so fail the render rather than paper over a
            // hole in the page.
            FromForkServer::TileUnchanged(header) => {
                let Some(kept) = known_tiles.get(header.content_hash) else {
                    anyhow::bail!("renderer skipped a tile we do not have (hash {})", header.content_hash);
                };
                received.push(PageTile::Reused { header, kept });
            }
            FromForkServer::PageRendered { summary, hit_regions } => {
                return Ok(RenderedPage {
                    summary,
                    tiles: received,
                    hit_regions,
                })
            }
            FromForkServer::Refused(reason) => anyhow::bail!("{reason}"),
            other => anyhow::bail!("unexpected render-exchange message: {other:?}"),
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
                KeptTile {
                    width: header.width,
                    height: header.height,
                    format: header.format,
                    pixels: bytes::Bytes::copy_from_slice(mapping.as_slice()),
                },
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
    pub width: u32,
    pub height: u32,
    pub format: crate::fork_server::protocol::TileWireFormat,
    pub pixels: bytes::Bytes,
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
    pub fn render_page(
        &mut self,
        html: &str,
        url: &str,
        viewport: (f64, f64),
        loader: &dyn gosub_interface::resource_loader::ResourceLoader,
        known_tiles: &TileMemory,
        hovered_node: Option<u64>,
    ) -> anyhow::Result<RenderedPage> {
        self.link.send(&ToForkServer::RenderPage {
            html: html.to_string(),
            url: url.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            known_tiles: known_tiles.hashes(),
            hovered_node,
        })?;
        drive_render_exchange(&mut self.link, loader, known_tiles)
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
