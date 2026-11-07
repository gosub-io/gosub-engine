//! Tile raster window and cache memory budget with LRU eviction.
//!
//! Both halves work off one [`raster_window`]: tiles inside it are rasterized and never evicted,
//! tiles outside it are deferred ([`defer_tiles_outside_window`]) or evicted ([`TileBudget`]).
//! Sharing the window is what stops generation from producing tiles eviction instantly discards.
//!
//! Eviction only removes pixels and records where they were - restoring them is the caller's
//! decision, since per-tile re-rasterization is not always possible (e.g. remotely rendered
//! caches). Callers poll [`TileBudget::needs_rerender`] on scroll.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

use crate::common::geo::{Dimension, Rect};
use crate::common::texture::TilePixels;
use crate::rasterizer::{BakedTile, TilePixelCache};
use crate::render::backend::{anchored_tile_pos, TileAnchor};
use crate::tiler::{TileBand, TileList, TileState};

/// A tile's position identity: (page_x bits, page_y bits, layer id). One eviction unit covers
/// the baked tile at that position plus any pixel-cache entries for it.
pub type TilePosKey = (u64, u64, u64);

fn baked_pos_key(tile: &BakedTile) -> TilePosKey {
    (tile.page_x.to_bits(), tile.page_y.to_bits(), tile.layer_id)
}

/// Outcome of one [`TileBudget::enforce`] pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct EvictionReport {
    /// Number of tile positions evicted.
    pub evicted_tiles: usize,
    pub evicted_bytes: usize,
    /// Pixel bytes still resident after enforcement. Can exceed the budget when the protected
    /// near-viewport band alone is larger than the budget.
    pub resident_bytes: usize,
}

#[derive(Default)]
struct BudgetState {
    /// Monotonic composite counter used as the LRU clock.
    tick: u64,
    last_used: HashMap<TilePosKey, u64>,
    /// Page-space rects whose tiles were evicted; scrolling near one requires a re-render.
    evicted: Vec<Rect>,
    /// Page-space rect last rasterized, clamped to the page. `None` before first raster.
    rastered: Option<Rect>,
}

/// LRU bookkeeping and budget enforcement for one browsing context's tile caches.
/// Interior-mutable so composite paths that only hold `&self` can still stamp usage.
#[derive(Default)]
pub struct TileBudget {
    state: Mutex<BudgetState>,
}

/// Viewports rasterized on each side of the viewport, so scrolling reveals ready tiles.
pub const RASTER_MARGIN_VIEWPORTS: f64 = 1.0;

/// Viewports of slack before the window is re-rastered. Smaller than
/// [`RASTER_MARGIN_VIEWPORTS`], so the re-raster is scheduled while ready tiles remain ahead.
pub const REFRESH_MARGIN_VIEWPORTS: f64 = 0.5;

/// The rasterized rows in page space.
pub fn raster_window(scroll_y: f64, vp_h: f64) -> (f64, f64) {
    let margin = RASTER_MARGIN_VIEWPORTS * vp_h;
    (scroll_y - margin, scroll_y + vp_h + margin)
}

/// The rasterized columns in page space. Bounded like the rows: one unbroken line can make a
/// page millions of pixels wide.
pub fn raster_columns(scroll_x: f64, vp_w: f64) -> (f64, f64) {
    let margin = RASTER_MARGIN_VIEWPORTS * vp_w;
    (scroll_x - margin, scroll_x + vp_w + margin)
}

/// The rows a pass around `scroll_y` tiles: the [`raster_window`]. Rows outside it get no tile;
/// columns outside [`raster_columns`] get one, but [`defer_tiles_outside_window`] parks it.
pub fn window_band(scroll_y: f64, vp_h: f64) -> TileBand {
    let (lo, hi) = raster_window(scroll_y, vp_h);
    TileBand::Rows { lo, hi, vp_h }
}

/// The page-space rect rasterized around `view`, the viewport in page space. A viewport with
/// no width yet keeps every column.
fn raster_rect(view: Rect) -> Rect {
    let (x0, x1) = if view.width > 0.0 {
        raster_columns(view.x, view.width)
    } else {
        (f64::MIN, f64::MAX)
    };
    let (y0, y1) = raster_window(view.y, view.height);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// The rect that must be resident before the next frame; escaping it triggers a re-raster.
fn refresh_rect(view: Rect) -> Rect {
    let mx = REFRESH_MARGIN_VIEWPORTS * view.width;
    let my = REFRESH_MARGIN_VIEWPORTS * view.height;
    Rect::new(view.x - mx, view.y - my, view.width + 2.0 * mx, view.height + 2.0 * my)
}

fn clamp_to_page(window: Rect, page: Dimension) -> Rect {
    let x0 = window.x.max(0.0);
    let y0 = window.y.max(0.0);
    let x1 = (window.x + window.width).min(page.width.max(0.0)).max(x0);
    let y1 = (window.y + window.height).min(page.height.max(0.0)).max(y0);
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y
}

/// Park dirty tiles outside the raster window around `view` (the viewport in page space) in
/// [`TileState::Deferred`] so stages 5 and 6 skip them; returns how many were parked.
/// Viewport-pinned layers are exempt: their page positions do not track scroll, so a page-space
/// window says nothing about whether they are on screen.
pub fn defer_tiles_outside_window(tile_list: &mut TileList, view: Rect) -> usize {
    defer_tiles_outside_band(tile_list, raster_rect(view))
}

/// [`defer_tiles_outside_window`] for an explicit page-space rect. A resize rasterizes only the
/// viewport itself, so the band is the viewport.
pub fn defer_tiles_outside_band(tile_list: &mut TileList, band: Rect) -> usize {
    let layer_list = std::sync::Arc::clone(&tile_list.layer_list);
    let mut deferred = 0;

    for tile in tile_list.arena.values_mut() {
        if tile.state != TileState::Dirty {
            continue;
        }
        if layer_list.layer_anchor(tile.layer_id) != TileAnchor::Scroll {
            continue;
        }
        if overlaps(tile.rect, band) {
            continue;
        }
        tile.state = TileState::Deferred;
        deferred += 1;
    }

    deferred
}

impl TileBudget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all bookkeeping. Call when the tile grid is rebuilt from scratch
    /// (new document, viewport resize).
    pub fn reset(&self) {
        let mut st = self.state.lock();
        st.tick = 0;
        st.last_used.clear();
        st.evicted.clear();
        st.rastered = None;
    }

    /// Every tile was just re-rasterized, so previously evicted regions are live again.
    pub fn note_full_raster(&self) {
        self.state.lock().evicted.clear();
    }

    /// Record the window around `view` as rasterized; [`Self::needs_rerender`] measures
    /// against it.
    pub fn note_rastered_window(&self, view: Rect, page: Dimension) {
        self.state.lock().rastered = Some(clamp_to_page(raster_rect(view), page));
    }

    /// Record an explicit page-space rect as rasterized, when a render covered less than the
    /// window around the viewport: a resize pass rasterizes the viewport alone.
    pub fn note_rastered_band(&self, band: Rect, page: Dimension) {
        self.state.lock().rastered = Some(clamp_to_page(band, page));
    }

    /// The page-space rows last noted as rasterized, if any. Scrolling inside them composites
    /// what is baked without a new pass, so a pass that repaints in place has to cover them.
    pub fn rastered_band(&self) -> Option<(f64, f64)> {
        self.state.lock().rastered.map(|r| (r.y, r.y + r.height))
    }

    /// Stamp the tiles visible at the given scroll position as most-recently composited.
    pub fn touch_composited(&self, tiles: &[BakedTile], scroll_x: f64, scroll_y: f64, vp_w: f64, vp_h: f64) {
        let mut st = self.state.lock();
        st.tick += 1;
        let tick = st.tick;
        for tile in tiles {
            let (ex, ey) = anchored_tile_pos(tile.page_x, tile.page_y, scroll_x, scroll_y, tile.anchor);
            let visible = ex < vp_w && ex + tile.width as f64 > 0.0 && ey < vp_h && ey + tile.height as f64 > 0.0;
            if !visible {
                continue;
            }
            st.last_used.insert(baked_pos_key(tile), tick);
        }
    }

    /// True when content the viewport is about to need was never rastered or has been evicted.
    /// Compositing cannot conjure either back, so the caller must re-raster the window.
    pub fn needs_rerender(&self, view: Rect, page: Dimension) -> bool {
        let need = clamp_to_page(refresh_rect(view), page);
        let st = self.state.lock();

        if st.evicted.iter().any(|r| overlaps(*r, need)) {
            return true;
        }

        // Tolerance keeps sub-pixel scroll jitter at a page edge from re-rastering every frame.
        const EPS: f64 = 0.5;
        match st.rastered {
            None => false,
            Some(r) => {
                need.x < r.x - EPS
                    || need.x + need.width > r.x + r.width + EPS
                    || need.y < r.y - EPS
                    || need.y + need.height > r.y + r.height + EPS
            }
        }
    }

    /// Evict least-recently-composited tile positions until resident pixel bytes fit
    /// `budget_bytes` (`0` disables enforcement).
    ///
    /// Never evicted: tiles overlapping the raster window around `view`, fixed/sticky tiles
    /// (viewport-pinned regardless of scroll), and GPU-resident tiles
    /// (their memory lives in the backend's texture store, not here). Ties in LRU age evict
    /// the position farthest from the viewport first.
    pub fn enforce(
        &self,
        tiles: &mut Vec<BakedTile>,
        pixel_cache: &mut TilePixelCache,
        view: Rect,
        budget_bytes: usize,
    ) -> EvictionReport {
        struct PosInfo {
            bytes: usize,
            protected: bool,
            rect: Rect,
        }

        let mut st = self.state.lock();
        let band = raster_rect(view);

        // Aggregate resident bytes per tile position. Pixel buffers are `Bytes` and shared
        // between a BakedTile, its CachedTile and its pixel-cache entry, so identical buffers
        // (by pointer + length) are counted once.
        let mut counted: HashSet<(usize, usize)> = HashSet::new();
        let mut positions: HashMap<TilePosKey, PosInfo> = HashMap::new();
        {
            let mut add = |key: TilePosKey, rect: Rect, pixels: &TilePixels, pinned: bool| {
                let (bytes, gpu) = match pixels {
                    TilePixels::Cpu(d) => {
                        let fresh = counted.insert((d.as_ptr() as usize, d.len()));
                        (if fresh { d.len() } else { 0 }, false)
                    }
                    TilePixels::Gpu(_) => (0, true),
                };
                let in_band = overlaps(rect, band);
                let entry = positions.entry(key).or_insert(PosInfo {
                    bytes: 0,
                    protected: false,
                    rect,
                });
                entry.bytes += bytes;
                entry.protected |= pinned || gpu || in_band;
            };

            for tile in tiles.iter() {
                let rect = Rect::new(tile.page_x, tile.page_y, tile.width as f64, tile.height as f64);
                let pinned = !matches!(tile.anchor, TileAnchor::Scroll);
                add(baked_pos_key(tile), rect, &tile.pixels, pinned);
            }
            for (key, (w, h, pixels)) in pixel_cache.iter() {
                let rect = Rect::new(f64::from_bits(key.0), f64::from_bits(key.1), *w as f64, *h as f64);
                add((key.0, key.1, key.2), rect, pixels, false);
            }
        }

        // Drop stamps for positions that no longer exist so the LRU map cannot grow unbounded.
        st.last_used.retain(|k, _| positions.contains_key(k));

        let resident: usize = positions.values().map(|p| p.bytes).sum();
        let mut report = EvictionReport {
            resident_bytes: resident,
            ..Default::default()
        };
        if budget_bytes == 0 || resident <= budget_bytes {
            return report;
        }

        // Oldest tick first (never-composited tiles carry tick 0), then farthest from the
        // viewport, then position key for determinism.
        let (cx, cy) = (view.x + view.width / 2.0, view.y + view.height / 2.0);
        let mut candidates: Vec<(u64, f64, TilePosKey, usize, Rect)> = positions
            .iter()
            .filter(|(_, p)| !p.protected && p.bytes > 0)
            .map(|(k, p)| {
                let dist = (p.rect.x + p.rect.width / 2.0 - cx).hypot(p.rect.y + p.rect.height / 2.0 - cy);
                (st.last_used.get(k).copied().unwrap_or(0), dist, *k, p.bytes, p.rect)
            })
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));

        let mut excess = resident - budget_bytes;
        let mut kill: HashSet<TilePosKey> = HashSet::new();
        for (_, _, key, bytes, rect) in candidates {
            if excess == 0 {
                break;
            }
            kill.insert(key);
            st.evicted.push(rect);
            report.evicted_bytes += bytes;
            excess = excess.saturating_sub(bytes);
        }

        report.evicted_tiles = kill.len();
        report.resident_bytes = resident - report.evicted_bytes;

        tiles.retain(|t| !kill.contains(&baked_pos_key(t)));
        pixel_cache.retain(|k, _| !kill.contains(&(k.0, k.1, k.2)));
        st.last_used.retain(|k, _| !kill.contains(k));

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::backend::PixelFormat;

    const TILE: f64 = 256.0;
    const TILE_BYTES: usize = 256 * 256 * 4;

    fn tile(x: f64, y: f64, layer: u64) -> BakedTile {
        BakedTile {
            page_x: x,
            page_y: y,
            layer_id: layer,
            width: 256,
            height: 256,
            pixels: TilePixels::Cpu(bytes::Bytes::from(vec![0u8; TILE_BYTES])),
            format: PixelFormat::PreMulArgb32,
            opacity: 1.0,
            anchor: TileAnchor::Scroll,
        }
    }

    /// A column of `n` tiles stacked from y=0.
    fn column(n: usize) -> Vec<BakedTile> {
        (0..n).map(|i| tile(0.0, i as f64 * TILE, 0)).collect()
    }

    /// A viewport one tile wide at the left edge of the page, scrolled to `y`.
    fn view(y: f64, height: f64) -> Rect {
        Rect::new(0.0, y, TILE, height)
    }

    /// A page one tile wide.
    fn page(height: f64) -> Dimension {
        Dimension::new(TILE, height)
    }

    fn has_tile_at(tiles: &[BakedTile], y: f64) -> bool {
        tiles.iter().any(|t| t.page_y == y)
    }

    #[test]
    fn budget_zero_disables_eviction() {
        let budget = TileBudget::new();
        let mut tiles = column(8);
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 0);

        assert_eq!(report.evicted_tiles, 0);
        assert_eq!(tiles.len(), 8);
        assert_eq!(report.resident_bytes, 8 * TILE_BYTES);
    }

    #[test]
    fn budget_respected_and_band_survives() {
        let budget = TileBudget::new();
        // Viewport is one tile tall at y=0 -> band [-256, 512] protects rows 0 and 1.
        let mut tiles = column(8);
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 3 * TILE_BYTES);

        assert!(report.resident_bytes <= 3 * TILE_BYTES);
        assert_eq!(report.evicted_tiles, 5);
        assert!(has_tile_at(&tiles, 0.0));
        assert!(has_tile_at(&tiles, TILE));
        // Farthest-first among equally-old tiles: the bottom rows go, row 2 stays.
        assert!(has_tile_at(&tiles, 2.0 * TILE));
        assert!(!has_tile_at(&tiles, 7.0 * TILE));
    }

    #[test]
    fn viewport_band_never_evicted_even_over_budget() {
        let budget = TileBudget::new();
        let mut tiles = column(2); // both rows inside the band
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 1);

        assert_eq!(report.evicted_tiles, 0);
        assert_eq!(tiles.len(), 2);
        assert!(report.resident_bytes > 1);
    }

    #[test]
    fn lru_order_evicts_least_recently_composited_first() {
        let budget = TileBudget::new();
        // Three tiles far below the viewport, equidistant enough that LRU dominates.
        let mut tiles = vec![tile(0.0, 4000.0, 0), tile(0.0, 4256.0, 0), tile(0.0, 4512.0, 0)];
        let mut cache = TilePixelCache::new();

        // Composite passes at their scroll positions: y=4256 last used most recently,
        // y=4000 before that, y=4512 never.
        budget.touch_composited(&tiles, 0.0, 4000.0, TILE, TILE);
        budget.touch_composited(&tiles, 0.0, 4256.0, TILE, TILE);

        // Viewport back at the top; budget keeps one of the three.
        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), TILE_BYTES);

        assert_eq!(report.evicted_tiles, 2);
        assert!(has_tile_at(&tiles, 4256.0), "most recently composited must survive");
        assert!(!has_tile_at(&tiles, 4512.0), "never-composited goes first");
        assert!(!has_tile_at(&tiles, 4000.0));
    }

    #[test]
    fn pixel_cache_follows_evicted_tiles_and_shared_buffers_count_once() {
        let budget = TileBudget::new();
        let far = tile(0.0, 4000.0, 0);
        let near = tile(0.0, 0.0, 0);
        let mut cache = TilePixelCache::new();
        // Cache entries share the baked tiles' pixel buffers, as rasterize_parallel produces.
        cache.insert(
            (far.page_x.to_bits(), far.page_y.to_bits(), 0, 1),
            (256, 256, far.pixels.clone()),
        );
        cache.insert(
            (near.page_x.to_bits(), near.page_y.to_bits(), 0, 2),
            (256, 256, near.pixels.clone()),
        );
        let mut tiles = vec![near, far];

        // Shared buffers: resident is 2 tiles' worth, not 4.
        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 10 * TILE_BYTES);
        assert_eq!(report.resident_bytes, 2 * TILE_BYTES);

        // Force eviction of the far tile; its pixel-cache entry must go with it.
        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), TILE_BYTES);
        assert_eq!(report.evicted_tiles, 1);
        assert!(!has_tile_at(&tiles, 4000.0));
        assert_eq!(cache.len(), 1);
        assert!(cache.keys().all(|k| f64::from_bits(k.1) == 0.0));
    }

    #[test]
    fn pixel_cache_only_positions_are_counted_and_evictable() {
        let budget = TileBudget::new();
        let mut tiles: Vec<BakedTile> = Vec::new();
        let mut cache = TilePixelCache::new();
        let orphan = tile(0.0, 4000.0, 0);
        cache.insert(
            (0.0f64.to_bits(), 4000.0f64.to_bits(), 0, 9),
            (256, 256, orphan.pixels.clone()),
        );

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 1);

        assert_eq!(report.evicted_tiles, 1);
        assert!(cache.is_empty());
    }

    #[test]
    fn fixed_and_gpu_tiles_are_never_evicted() {
        let budget = TileBudget::new();
        let mut fixed = tile(0.0, 4000.0, 1);
        fixed.anchor = TileAnchor::Fixed;
        let gpu = BakedTile {
            pixels: TilePixels::Gpu(crate::common::texture::GpuTile::untracked(7)),
            ..tile(0.0, 4256.0, 2)
        };
        let mut tiles = vec![fixed, gpu];
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 1);

        assert_eq!(report.evicted_tiles, 0);
        assert_eq!(tiles.len(), 2);
    }

    #[test]
    fn needs_rerender_tracks_evicted_regions() {
        const PAGE: f64 = 8000.0;
        let budget = TileBudget::new();
        let mut tiles = column(2);
        tiles.push(tile(0.0, 4000.0, 0));
        let mut cache = TilePixelCache::new();

        // Pretend the whole page was rastered, so only eviction can create a hole.
        budget.note_rastered_window(view(PAGE / 2.0, PAGE), page(PAGE));
        budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 2 * TILE_BYTES);
        assert!(!has_tile_at(&tiles, 4000.0));

        assert!(
            !budget.needs_rerender(view(0.0, TILE), page(PAGE)),
            "evicted region is far away"
        );
        assert!(
            budget.needs_rerender(view(3900.0, TILE), page(PAGE)),
            "refresh window reaches the evicted region"
        );

        budget.note_full_raster();
        assert!(
            !budget.needs_rerender(view(3900.0, TILE), page(PAGE)),
            "full re-raster restores everything"
        );
    }

    #[test]
    fn needs_rerender_follows_the_rastered_window() {
        const PAGE: f64 = 10_000.0;
        let budget = TileBudget::new();

        // Nothing rastered yet: the caller is about to render anyway, so don't ask for another.
        assert!(!budget.needs_rerender(view(0.0, TILE), page(PAGE)));

        // Rastered around the top: window [0, 512]; the refresh window at scroll 0 is [0, 384].
        budget.note_rastered_window(view(0.0, TILE), page(PAGE));
        assert!(
            !budget.needs_rerender(view(0.0, TILE), page(PAGE)),
            "fresh window needs nothing"
        );
        assert!(
            !budget.needs_rerender(view(100.0, TILE), page(PAGE)),
            "small scroll stays inside the slack"
        );
        assert!(
            budget.needs_rerender(view(300.0, TILE), page(PAGE)),
            "scrolling past the slack must extend the window"
        );

        // Extending around the new position satisfies the demand again.
        budget.note_rastered_window(view(300.0, TILE), page(PAGE));
        assert!(!budget.needs_rerender(view(300.0, TILE), page(PAGE)));
    }

    #[test]
    fn page_edges_do_not_re_raster_forever() {
        const PAGE: f64 = 1000.0;
        let budget = TileBudget::new();

        // At either page edge the windows run past the page; clamping both to the page is what
        // stops a permanent "needs re-raster" there.
        let bottom = PAGE - TILE;
        budget.note_rastered_window(view(bottom, TILE), page(PAGE));
        assert!(!budget.needs_rerender(view(bottom, TILE), page(PAGE)));

        budget.note_rastered_window(view(0.0, TILE), page(PAGE));
        assert!(!budget.needs_rerender(view(0.0, TILE), page(PAGE)));
    }

    #[test]
    fn needs_rerender_follows_a_horizontal_scroll() {
        let budget = TileBudget::new();
        let page = Dimension::new(1_000_000.0, TILE);
        let at = |x: f64| Rect::new(x, 0.0, TILE, TILE);

        // Rastered columns [0, 512]; the refresh window at x 0 is [0, 384].
        budget.note_rastered_window(at(0.0), page);
        assert!(
            !budget.needs_rerender(at(100.0), page),
            "small scroll stays inside the slack"
        );
        assert!(budget.needs_rerender(at(300.0), page), "scrolling right past the slack");

        budget.note_rastered_window(at(300.0), page);
        assert!(!budget.needs_rerender(at(300.0), page));
    }

    #[test]
    fn tiles_left_and_right_of_the_window_are_evictable() {
        let budget = TileBudget::new();
        let mut tiles = vec![tile(0.0, 0.0, 0), tile(10.0 * TILE, 0.0, 0)];
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, view(0.0, TILE), 1);

        assert_eq!(report.evicted_tiles, 1);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].page_x, 0.0, "the tile in the window stays");
    }

    /// Tiles a real 6 000 px layout, so the deferral policy is exercised against the actual
    /// `TileList` the pipeline builds rather than a hand-made stand-in.
    mod deferral {
        use super::*;
        use crate::common::document::pipeline_doc::GosubDocumentAdapter;
        use crate::layering::layer::LayerList;
        use crate::layouter::taffy::TaffyLayouter;
        use crate::layouter::CanLayout;
        use crate::rendertree_builder::RenderTree;
        use crate::tiler::TileList;
        use gosub_css3::system::Css3System;
        use gosub_html5::document::document_impl::DocumentImpl;
        use gosub_html5::html_compile;
        use gosub_html5::parser::Html5Parser;
        use gosub_interface::config::ModuleConfiguration;
        use gosub_interface::css3::CssSystem as _;
        use gosub_interface::document::Document as _;
        use std::sync::Arc;

        #[derive(Clone, Debug, PartialEq)]
        struct Config;

        impl ModuleConfiguration for Config {
            type CssSystem = Css3System;
            type Document = DocumentImpl<Self>;
            type HtmlParser = Html5Parser<'static, Self>;
        }

        /// `html` laid out 512 px wide, with an empty grid.
        fn page_grid(html: &str) -> TileList {
            let mut doc = html_compile::<Config>(html);
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());

            let mut render_tree = RenderTree::new(Arc::new(GosubDocumentAdapter::<Config>::new(Arc::new(doc))));
            let _ = render_tree.parse();

            let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(512.0, TILE)), 1.0);
            TileList::new(LayerList::new(Arc::new(layout_tree)), Dimension::new(TILE, TILE))
        }

        const TALL_PAGE: &str =
            r#"<html><body style="margin:0"><div style="height:6000px;background:#ccc"></div></body></html>"#;

        fn tall_page_tiles() -> TileList {
            let mut tile_list = page_grid(TALL_PAGE);
            tile_list.generate();
            tile_list
        }

        /// A tile's layer and position, and per element its id and placement in the tile.
        type TileShape = (u64, u64, u64, Vec<(String, [u64; 6])>);

        /// What a tile is, minus its id: where it is and what it paints.
        fn tile_shape(tile: &crate::tiler::Tile) -> TileShape {
            let elements = tile
                .elements
                .iter()
                .map(|e| {
                    let placement = [
                        e.rect.x,
                        e.rect.y,
                        e.rect.width,
                        e.rect.height,
                        e.position.x,
                        e.position.y,
                    ];
                    (format!("{:?}", e.id), placement.map(f64::to_bits))
                })
                .collect();
            (
                tile.rect.x.to_bits(),
                tile.rect.y.to_bits(),
                tile.layer_id.as_u64(),
                elements,
            )
        }

        #[test]
        fn a_huge_page_only_gets_tiles_for_the_window() {
            // #1393: one tile per cell of the page was ~19.5M tiles for this page.
            let mut tile_list = page_grid(
                r#"<html><body style="margin:0"><div style="height:1000000000px;background:#ccc"></div></body></html>"#,
            );
            let page_h = tile_list.layer_list.layout_tree.root_dimension.height;
            assert!(page_h > 1.0e8, "the page must really be that tall, got {page_h}");

            let (lo, hi) = raster_window(0.0, TILE);
            let window_rows = ((hi - lo) / TILE).ceil() as usize + 1;
            let window_tiles = window_rows * (512.0 / TILE) as usize * tile_list.layer_list.layer_ids.read().len();

            tile_list.cover(window_band(0.0, TILE));
            let at_top = tile_list.arena.len();
            assert!(at_top > 0, "the window must get tiles");
            assert!(
                at_top <= window_tiles,
                "{at_top} tiles for a window of at most {window_tiles}"
            );

            // Deep down the page: the same handful, and none left behind at the top.
            let scroll = 5.0e8;
            tile_list.cover(window_band(scroll, TILE));
            assert!(
                tile_list.arena.len() <= window_tiles,
                "{} tiles after a jump",
                tile_list.arena.len()
            );
            let (lo, hi) = raster_window(scroll, TILE);
            assert!(
                tile_list
                    .arena
                    .values()
                    .all(|t| t.rect.y < hi && t.rect.y + t.rect.height > lo),
                "every tile must lie in the window around the new scroll position"
            );
        }

        #[test]
        fn covering_a_band_tiles_it_as_the_whole_page_would() {
            let full = tall_page_tiles();
            let mut whole: Vec<_> = full.arena.values().map(tile_shape).collect();
            whole.sort();

            let mut banded = page_grid(TALL_PAGE);
            banded.cover(window_band(3000.0, TILE));
            let (lo, hi) = raster_window(3000.0, TILE);
            let mut expected: Vec<_> = full
                .arena
                .values()
                .filter(|t| t.rect.y < hi && t.rect.y + t.rect.height > lo)
                .map(tile_shape)
                .collect();
            expected.sort();
            let mut got: Vec<_> = banded.arena.values().map(tile_shape).collect();
            got.sort();

            assert!(!got.is_empty());
            assert!(got.len() < whole.len());
            assert_eq!(
                got, expected,
                "a banded tile must be the tile a whole-page grid has there"
            );
        }

        #[test]
        fn moving_the_band_keeps_the_tiles_it_still_covers() {
            let mut tile_list = page_grid(TALL_PAGE);
            tile_list.cover(window_band(2000.0, TILE));
            let before: std::collections::HashMap<_, _> = tile_list
                .arena
                .values()
                .map(|t| ((t.rect.x.to_bits(), t.rect.y.to_bits(), t.layer_id), t.id))
                .collect();

            tile_list.cover(window_band(2000.0 + TILE, TILE));
            let (lo, hi) = raster_window(2000.0 + TILE, TILE);
            let mut kept = 0;
            for tile in tile_list.arena.values() {
                assert!(tile.rect.y < hi && tile.rect.y + tile.rect.height > lo);
                if let Some(&id) = before.get(&(tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id)) {
                    assert_eq!(id, tile.id, "a tile still in the band must be the same tile");
                    kept += 1;
                }
            }
            assert!(kept > 0, "the bands overlap, so some tiles must carry over");
            assert!(kept < before.len(), "the row scrolled out must be dropped");
        }

        fn count(tile_list: &TileList, state: TileState) -> usize {
            tile_list.arena.values().filter(|t| t.state == state).count()
        }

        #[test]
        fn defers_tiles_outside_the_window_only() {
            let mut tile_list = tall_page_tiles();
            let total = tile_list.arena.len();
            assert!(total > 20, "a 6 000 px page must produce many tiles, got {total}");

            let deferred = defer_tiles_outside_window(&mut tile_list, Rect::new(0.0, 0.0, 512.0, TILE));
            let dirty = count(&tile_list, TileState::Dirty);

            assert_eq!(deferred + dirty, total, "every tile is either deferred or paintable");
            assert!(dirty > 0, "the window itself must stay paintable");
            assert!(
                deferred > dirty,
                "far more of a tall page is out of window than in it ({deferred} vs {dirty})"
            );

            // Everything still dirty must intersect the window; everything deferred must not.
            let (lo, hi) = raster_window(0.0, TILE);
            for tile in tile_list.arena.values() {
                let in_window = tile.rect.y < hi && tile.rect.y + tile.rect.height > lo;
                match tile.state {
                    TileState::Dirty => assert!(in_window, "kept an out-of-window tile at y={}", tile.rect.y),
                    TileState::Deferred => assert!(!in_window, "deferred an in-window tile at y={}", tile.rect.y),
                    _ => {}
                }
            }
        }

        #[test]
        fn deferral_follows_the_scroll_position() {
            let mut tile_list = tall_page_tiles();
            defer_tiles_outside_window(&mut tile_list, Rect::new(0.0, 3000.0, 512.0, TILE));

            let (lo, hi) = raster_window(3000.0, TILE);
            let dirty_ys: Vec<f64> = tile_list
                .arena
                .values()
                .filter(|t| t.state == TileState::Dirty)
                .map(|t| t.rect.y)
                .collect();

            assert!(!dirty_ys.is_empty(), "the window around y=3000 must hold tiles");
            assert!(
                dirty_ys.iter().all(|&y| y + TILE > lo && y < hi),
                "only tiles around the scrolled viewport stay paintable: {dirty_ys:?}"
            );
        }

        /// One unbroken line makes the page far wider than the viewport; only the columns
        /// around it stay paintable (GOS-285).
        #[test]
        fn defers_tiles_beside_the_window() {
            let mut tile_list = page_grid(&format!("<p>{}</p>", "x".repeat(20_000)));
            tile_list.cover(window_band(0.0, TILE));
            let page_width = tile_list.layer_list.layout_tree.root_dimension.width;
            assert!(page_width > 20.0 * 512.0, "the line overflows: {page_width}");

            defer_tiles_outside_window(&mut tile_list, Rect::new(0.0, 0.0, 512.0, TILE));

            let (lo, hi) = raster_columns(0.0, 512.0);
            let dirty_xs: Vec<f64> = tile_list
                .arena
                .values()
                .filter(|t| t.state == TileState::Dirty)
                .map(|t| t.rect.x)
                .collect();
            assert!(!dirty_xs.is_empty());
            assert!(
                dirty_xs.iter().all(|&x| x + TILE > lo && x < hi),
                "only columns around the viewport stay paintable: {dirty_xs:?}"
            );
            assert!(count(&tile_list, TileState::Deferred) > 30);
        }
    }
}
