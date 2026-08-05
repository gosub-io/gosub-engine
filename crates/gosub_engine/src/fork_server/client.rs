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

/// One rasterized tile as the broker holds it: the wire header plus the
/// mapped, validated, immutable pixels — the renderer's pages, not a copy.
#[derive(Debug)]
pub struct ReceivedTile {
    pub header: crate::fork_server::protocol::TileHeader,
    pub mapping: gosub_ipc::shm::TileMapping,
}

impl ReceivedTile {
    /// Hand this tile to the compositor: the exact [`CachedTile`] shape the
    /// host-side tile compositing loop consumes — **still zero-copy**. The
    /// mapping becomes the `Bytes`' owner (`Bytes::from_owner`), so the
    /// compositor blends straight out of the renderer's sealed pages and the
    /// mapping is unmapped when the last tile reference drops.
    pub fn into_cached_tile(self) -> gosub_interface::render::backend::CachedTile {
        // Alpha is the 4th byte in both supported formats ([B,G,R,A] / [R,G,B,A]).
        let opaque = self.mapping.as_slice().chunks_exact(4).all(|px| px[3] == 0xFF);
        gosub_interface::render::backend::CachedTile {
            page_x: self.header.page_x as f32,
            page_y: self.header.page_y as f32,
            width: self.header.width,
            height: self.header.height,
            data: bytes::Bytes::from_owner(self.mapping),
            format: self.header.format.into(),
            opacity: self.header.opacity,
            anchor: self.header.anchor.into(),
            opaque,
        }
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
            true,
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

    /// The confinement tier the configured font system answered — what decides
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

    /// Fork a renderer and run the pipeline over `html` in it — parse, style,
    /// layout, layering, tiling, paint, and (when the configuration has a
    /// forked rasterizer) rasterize — under its tier sandbox, with the
    /// inherited fonts. Returns the measured summary plus the rasterized
    /// tiles, whose pixels arrive as sealed memfds and are mapped — never
    /// copied — into this process.
    pub fn render_page(
        &mut self,
        html: &str,
        viewport: (f64, f64),
        loader: &dyn gosub_interface::resource_loader::ResourceLoader,
    ) -> anyhow::Result<(crate::fork_server::protocol::PageSummary, Vec<ReceivedTile>)> {
        use crate::fork_server::protocol::ResourceReply;

        self.link.send(&ToForkServer::RenderPage {
            html: html.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
        })?;
        loop {
            match self.link.recv::<FromForkServer>()? {
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
                    self.link.send(&ToForkServer::Resource(reply))?;
                }
                FromForkServer::PageRendered { summary, tiles } => {
                    let mut received = Vec::with_capacity(tiles.len());
                    for header in tiles {
                        let fd = self.link.rx.recv_fd()?;
                        let mapping = gosub_ipc::shm::map_sealed_tile(fd, header.width, header.height)?;
                        received.push(ReceivedTile { header, mapping });
                    }
                    return Ok((summary, received));
                }
                FromForkServer::Refused(reason) => anyhow::bail!("{reason}"),
                other => anyhow::bail!("unexpected reply to RenderPage: {other:?}"),
            }
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
