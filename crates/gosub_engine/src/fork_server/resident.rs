//! The resident renderer: a forked, confined child that stays alive and
//! renders for every tab of one (zone, site) until its last tab closes.
//!
//! Same pipeline as the one-shot renderer (`renderer::render_page`), run in a
//! loop instead of once - the step from "a process per render" to "a process
//! per site", with nothing retained between renders yet.

use crate::fork_server::loader::ForkedResourceLoader;
use crate::fork_server::protocol::{FromRenderer, HitRegion, PageSummary, TileHeader, ToRenderer};
use crate::fork_server::renderer::{self, RenderedTile};
use crate::html::RenderConfiguration;
use gosub_interface::font_system::FontSystem;
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::os::fd::AsRawFd;
use std::sync::Arc;

/// The `comm` (15-byte `ps` name) for a renderer labelled `label`.
pub(super) fn comm_for(label: &str) -> String {
    let short: String = label.chars().take(8).collect();
    if short.is_empty() {
        "render".to_string()
    } else {
        format!("render-{short}")
    }
}

/// Serve [`ToRenderer`] requests over `link` until told to stop or the link
/// closes. Runs in a forked child that has already confined itself; never
/// returns, since a forked child must leave through `exit_now`.
pub fn serve<C: RenderConfiguration>(
    link: Endpoint,
    fonts: C::FontSystem,
    media_store: Arc<gosub_render_pipeline::common::media::MediaStore>,
    forked_loader: Arc<ForkedResourceLoader>,
    label: &str,
) -> ! {
    // Shared between the loader (mid-render round trips) and the request
    // loop - safe because this process is single-threaded and strictly
    // alternates: nothing else touches the link while a render runs.
    let link = Arc::new(Mutex::new(link));
    forked_loader.connect(Arc::clone(&link));
    let fonts: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(fonts));
    let comm = comm_for(label);

    let mut tabs: HashSet<String> = HashSet::new();
    loop {
        let request = link.lock().recv::<ToRenderer>();
        let request = match request {
            Ok(request) => request,
            // The broker went away (or closed the link on purpose).
            Err(_) => gosub_sandbox::exit_now(0),
        };
        match request {
            ToRenderer::OpenTab { tab } => {
                tabs.insert(tab);
            }
            ToRenderer::CloseTab { tab } => {
                tabs.remove(&tab);
            }
            ToRenderer::Shutdown => gosub_sandbox::exit_now(0),
            ToRenderer::Navigate {
                tab: _,
                html,
                url,
                viewport_width,
                viewport_height,
                known_tiles,
                hovered_node,
            } => {
                gosub_sandbox::set_process_title(&comm, &format!("gosub: renderer {url}"));
                let known_tiles: HashSet<u64> = known_tiles.into_iter().collect();
                let (summary, tiles, hit_regions) = renderer::render_page::<C>(
                    renderer::PageRequest {
                        html: &html,
                        page_url: &url,
                        viewport_width,
                        viewport_height,
                        known_tiles: &known_tiles,
                        hovered_node,
                    },
                    Arc::clone(&fonts),
                    Arc::clone(&media_store),
                    Arc::clone(&forked_loader) as Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
                );
                if !stream_rendered(&mut link.lock(), &tiles, summary, hit_regions) {
                    gosub_sandbox::exit_now(1);
                }
            }
        }
    }
}

/// Stream a rendered page out over `link`: every CPU tile sealed into an
/// immutable memfd and sent as header + fd, one at a time (so this process
/// never holds more than one tile fd regardless of page height), then the
/// summary. All of it - memfd_create, ftruncate, mmap, the seals - is in the
/// renderer baseline precisely so a confined renderer can hand off pixels.
/// `false` means the link is gone and the caller should exit.
pub(super) fn stream_rendered(
    link: &mut Endpoint,
    tiles: &[RenderedTile],
    summary: PageSummary,
    hit_regions: Vec<HitRegion>,
) -> bool {
    for tile in tiles {
        let sent = match tile {
            RenderedTile::Unchanged {
                page_x,
                page_y,
                layer_id,
                hash,
            } => link
                .send(&FromRenderer::TileUnchanged(unchanged_header(
                    *page_x, *page_y, *layer_id, *hash,
                )))
                .is_ok(),
            RenderedTile::Fresh { tile, hash } => {
                let gosub_render_pipeline::common::texture::TilePixels::Cpu(bytes) = &tile.pixels else {
                    // GPU handles are process-local and cannot cross; a
                    // forked rasterizer should never produce one.
                    continue;
                };
                let Ok(fd) = gosub_ipc::shm::create_sealed_tile(tile.width, tile.height, |buf| {
                    let n = buf.len().min(bytes.len());
                    buf[..n].copy_from_slice(&bytes[..n]);
                }) else {
                    return false;
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
                link.send(&FromRenderer::Tile(header)).is_ok() && link.tx.send_fd(fd.as_raw_fd()).is_ok()
                // `fd` drops here: SCM_RIGHTS duplicated it onward.
            }
        };
        if !sent {
            return false;
        }
    }
    link.send(&FromRenderer::Rendered { summary, hit_regions }).is_ok()
}

/// The header for a tile the broker already holds. Its physical dimensions
/// are left at zero deliberately: nothing rasterized them here, and the
/// broker fills them from the pixels it kept.
fn unchanged_header(page_x: f64, page_y: f64, layer_id: u64, hash: u64) -> TileHeader {
    TileHeader {
        page_x,
        page_y,
        layer_id,
        width: 0,
        height: 0,
        format: crate::fork_server::protocol::TileWireFormat::PreMulArgb32,
        content_hash: hash,
        opacity: 1.0,
        anchor: crate::fork_server::protocol::TileWireAnchor::Scroll,
    }
}
