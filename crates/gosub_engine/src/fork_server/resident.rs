//! The resident renderer: a forked, confined child that stays alive and
//! renders for every tab of one (zone, site) until its last tab closes.
//!
//! Each tab's page is retained after `Navigate` (see
//! [`renderer::RetainedPage`]), so a `Scroll` only tiles, paints and
//! rasterizes what came into the raster window - no parse, no layout.

use crate::fork_server::loader::ForkedResourceLoader;
use crate::fork_server::protocol::{FromRenderer, HitRegion, PageSummary, TileHeader, ToRenderer};
use crate::fork_server::renderer::{self, RenderedTile, RetainedPage};
use crate::html::RenderConfiguration;
use gosub_interface::font_system::FontSystem;
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::os::fd::AsRawFd;
use std::sync::Arc;

/// Decoded media a resident renderer keeps between pages. Above this, a new
/// page starts with an empty media cache; the broker still holds the bytes.
const MEDIA_CACHE_BUDGET: usize = 64 * 1024 * 1024;

/// Retained pages one renderer holds at once. A heavy site with many tabs
/// would otherwise stack a laid-out page per tab until the process's memory
/// limit kills it; past the cap the least recently used tab's page is let go
/// of, and its next scroll comes back empty - which makes the broker render
/// it afresh.
const MAX_RETAINED_PAGES: usize = 3;

/// One incremental request, bound to run against a tab's retained page.
type PagePass = Box<dyn FnOnce(&mut RetainedPage) -> renderer::RenderPass>;

/// This renderer's `ps` name: `renderer-<6 hex>`, drawn once per process.
/// Deliberately not the site or tab: the process list is visible to every
/// user on the machine, and what a renderer renders is not their business.
pub(super) fn comm() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let id = uuid::Uuid::new_v4().simple().to_string();
        format!("renderer-{}", &id[..6])
    })
    .clone()
}

/// The cmdline to go with [`comm`].
pub(super) fn title() -> String {
    format!("gosub: {}", comm())
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
    let _ = label; // the pool's key; the process is named after itself
    let comm = comm();

    let mut pages: HashMap<String, RetainedPage> = HashMap::new();
    // Tab names, most recently used last; parallel to `pages`.
    let mut recent: Vec<String> = Vec::new();
    let touch = |recent: &mut Vec<String>, tab: &str| {
        recent.retain(|t| t != tab);
        recent.push(tab.to_string());
    };
    loop {
        let request = link.lock().recv::<ToRenderer>();
        let request = match request {
            Ok(request) => request,
            // The broker went away (or closed the link on purpose).
            Err(_) => gosub_sandbox::exit_now(0),
        };
        match request {
            ToRenderer::OpenTab { .. } => {}
            ToRenderer::CloseTab { tab } => {
                pages.remove(&tab);
                recent.retain(|t| t != &tab);
            }
            ToRenderer::Shutdown => gosub_sandbox::exit_now(0),
            ToRenderer::CrashForTest => gosub_sandbox::exit_now(139),
            ToRenderer::Navigate {
                tab,
                html,
                url,
                viewport_width,
                viewport_height,
                scroll_y,
                known_tiles,
                hovered_node,
            } => {
                let _ = &url;
                gosub_sandbox::set_process_title(&comm, &title());
                // A new page: what earlier pages decoded is dead weight past
                // the budget, and this process lives as long as the site's tabs.
                media_store.trim(MEDIA_CACHE_BUDGET);
                let known_tiles: HashSet<u64> = known_tiles.into_iter().collect();
                let mut page = RetainedPage::build::<C>(
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
                let pass = page.render(Some(scroll_y), &known_tiles);
                let hit_regions = page.hit_regions.clone();
                touch(&mut recent, &tab);
                pages.insert(tab, page);
                while pages.len() > MAX_RETAINED_PAGES {
                    let Some(oldest) = recent.first().cloned() else {
                        break;
                    };
                    recent.remove(0);
                    pages.remove(&oldest);
                }
                if !stream_rendered(&mut link.lock(), &pass.tiles, &pass.evicted, pass.summary, hit_regions) {
                    gosub_sandbox::exit_now(1);
                }
            }
            ToRenderer::Scroll { .. } | ToRenderer::Hover { .. } => {
                let (tab, run): (String, PagePass) = match request {
                    ToRenderer::Scroll { tab, scroll_y } => {
                        (tab, Box::new(move |page| page.render(Some(scroll_y), &HashSet::new())))
                    }
                    ToRenderer::Hover { tab, node } => (tab, Box::new(move |page| page.hover(node))),
                    _ => unreachable!("matched above"),
                };
                // A tab with no retained page (never navigated, or closed)
                // gets an empty pass, so the exchange still completes.
                let (pass, hit_regions) = match pages.get_mut(&tab) {
                    Some(page) => {
                        touch(&mut recent, &tab);
                        let pass = run(page);
                        (pass, page.hit_regions.clone())
                    }
                    None => (
                        renderer::RenderPass {
                            summary: PageSummary::default(),
                            tiles: Vec::new(),
                            evicted: Vec::new(),
                        },
                        Vec::new(),
                    ),
                };
                if !stream_rendered(&mut link.lock(), &pass.tiles, &pass.evicted, pass.summary, hit_regions) {
                    gosub_sandbox::exit_now(1);
                }
            }
        }
    }
}

/// Stream a render pass out over `link`: evictions first, then every CPU tile
/// sealed into an immutable memfd and sent as header + fd, one at a time (so
/// this process never holds more than one tile fd regardless of page height),
/// then the summary. All of it - memfd_create, ftruncate, mmap, the seals -
/// is in the renderer baseline precisely so a confined renderer can hand off
/// pixels. `false` means the link is gone and the caller should exit.
pub(super) fn stream_rendered(
    link: &mut Endpoint,
    tiles: &[RenderedTile],
    evicted: &[u64],
    summary: PageSummary,
    hit_regions: Vec<HitRegion>,
) -> bool {
    if !evicted.is_empty()
        && link
            .send(&FromRenderer::Evict {
                hashes: evicted.to_vec(),
            })
            .is_err()
    {
        return false;
    }
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
