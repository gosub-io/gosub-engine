//! The exec'd renderer role: set up, confine, render one page, exit.

use crate::fork_server::protocol::{FromForkServer, ResourceReply, TileHeader, ToForkServer};
use crate::fork_server::renderer;
use crate::html::RenderConfiguration;
use gosub_interface::font_system::{Confinement, FontSystem};
use gosub_interface::resource_loader::{LoadError, LoadedResource, ResourceLoader};
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::os::fd::AsRawFd;
use std::sync::Arc;
use url::Url;

/// The renderer's loader when it talks to the broker *directly* (no fork
/// server in between): `NeedResource` out, `Resource` back, on the same link
/// the render request arrived on. Blocking, one exchange in flight — the
/// renderer is single-threaded where it loads.
struct DirectBrokeredLoader {
    link: Arc<Mutex<Endpoint>>,
}

impl std::fmt::Debug for DirectBrokeredLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirectBrokeredLoader")
    }
}

impl ResourceLoader for DirectBrokeredLoader {
    fn load(&self, url: &Url) -> Result<LoadedResource, LoadError> {
        let mut link = self.link.lock();
        link.send(&FromForkServer::NeedResource { url: url.to_string() })
            .map_err(|e| LoadError::Failed(format!("could not reach the broker: {e}")))?;
        match link
            .recv::<ToForkServer>()
            .map_err(|e| LoadError::Failed(format!("the broker never answered: {e}")))?
        {
            ToForkServer::Resource(ResourceReply::Ok {
                status,
                content_type,
                body,
            }) => Ok(LoadedResource {
                status,
                content_type,
                body: bytes::Bytes::from(body),
            }),
            ToForkServer::Resource(ResourceReply::Failed(reason)) => Err(LoadError::Failed(reason)),
            other => Err(LoadError::Failed(format!("expected a resource, got {other:?}"))),
        }
    }
}

/// Render one page for the broker, then return the process exit code.
///
/// Everything that touches the filesystem beyond font paths — building the
/// font system (which may spawn a library worker thread; permitted, this
/// process kept its PID namespace and the filter is applied with TSYNC), the
/// media store's placeholder decode — happens before the lockdown; the parse
/// and render happen confined, reading only font paths and the private
/// scratch, fetching everything else through the broker.
pub fn serve<C: RenderConfiguration>(link: Endpoint) -> i32 {
    let mut fonts = C::FontSystem::default();
    let _ = fonts.families();
    let answer = fonts.prepare_for_confinement();
    if let Confinement::Unsupported(reason) = &answer {
        let mut link = link;
        let _ = link.send(&FromForkServer::Refused(format!(
            "this font system cannot run isolated: {reason}"
        )));
        return 1;
    }

    let link = Arc::new(Mutex::new(link));
    let loader: Arc<dyn ResourceLoader> = Arc::new(DirectBrokeredLoader {
        link: Arc::clone(&link),
    });
    let media_store = Arc::new(gosub_render_pipeline::common::media::MediaStore::with_loader(
        Arc::clone(&loader),
    ));
    media_store.set_synchronous_fetch(true);

    // The font-readable tier, whatever the instance answered: a `Full` system
    // routed here still works under the weaker profile, and this role exists
    // for the systems that need it.
    let scratch = std::env::temp_dir().join(format!("gosub-renderer-scratch-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scratch);
    std::env::set_var("TMPDIR", &scratch);
    let paths = gosub_sandbox::font_filesystem_paths();
    let mut refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
    refs.push((scratch.as_path(), true));
    gosub_sandbox::lock_down_renderer_with_font_access(&refs);

    // One request, one render, gone.
    let request = {
        let mut link = link.lock();
        match link.recv::<ToForkServer>() {
            Ok(msg) => msg,
            // The broker went away before asking; nothing to do.
            Err(_) => return 0,
        }
    };
    let ToForkServer::RenderPage {
        html,
        url,
        viewport_width,
        viewport_height,
        known_tiles,
    } = request
    else {
        let _ = link.lock().send(&FromForkServer::Refused(
            "an exec'd renderer serves exactly one RenderPage".into(),
        ));
        return 1;
    };

    let shared: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(fonts));
    let (summary, tiles, hit_regions) = renderer::render_page::<C>(
        renderer::PageRequest {
            html: &html,
            page_url: &url,
            viewport_width,
            viewport_height,
            known_tiles: &known_tiles.into_iter().collect(),
        },
        shared,
        media_store,
        loader,
    );

    // Stream the tiles out exactly as a forked renderer does: seal, send,
    // drop, one at a time.
    let mut link = link.lock();
    for tile in &tiles {
        let sent = match tile {
            renderer::RenderedTile::Unchanged {
                page_x,
                page_y,
                layer_id,
                hash,
            } => link
                .send(&FromForkServer::TileUnchanged(TileHeader {
                    page_x: *page_x,
                    page_y: *page_y,
                    layer_id: *layer_id,
                    // Zero dimensions: nothing rasterized here, so the broker
                    // fills them from the pixels it kept.
                    width: 0,
                    height: 0,
                    format: crate::fork_server::protocol::TileWireFormat::PreMulArgb32,
                    content_hash: *hash,
                    opacity: 1.0,
                    anchor: crate::fork_server::protocol::TileWireAnchor::Scroll,
                }))
                .is_ok(),
            renderer::RenderedTile::Fresh { tile, hash } => {
                let gosub_render_pipeline::common::texture::TilePixels::Cpu(bytes) = &tile.pixels else {
                    continue;
                };
                let Ok(fd) = gosub_ipc::shm::create_sealed_tile(tile.width, tile.height, |buf| {
                    let n = buf.len().min(bytes.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                }) else {
                    return 1;
                };
                let header = TileHeader {
                    page_x: tile.page_x,
                    page_y: tile.page_y,
                    layer_id: tile.layer_id,
                    width: tile.width,
                    height: tile.height,
                    format: tile.format.into(),
                    content_hash: *hash,
                    opacity: tile.opacity,
                    anchor: tile.anchor.into(),
                };
                link.send(&FromForkServer::Tile(header)).is_ok() && link.tx.send_fd(fd.as_raw_fd()).is_ok()
            }
        };
        if !sent {
            return 1;
        }
    }
    if link
        .send(&FromForkServer::PageRendered { summary, hit_regions })
        .is_err()
    {
        return 1;
    }
    0
}
