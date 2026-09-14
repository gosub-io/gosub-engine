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

use crate::common::geo::Rect;
use crate::common::texture::TilePixels;
use crate::rasterizer::{BakedTile, TilePixelCache};
use crate::render::backend::{anchored_tile_pos, TileAnchor};
use crate::tiler::{TileList, TileState};

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
    /// Page-space `(lo, hi)` band last rasterized, clamped to the page. `None` before first raster.
    rastered: Option<(f64, f64)>,
}

/// LRU bookkeeping and budget enforcement for one browsing context's tile caches.
/// Interior-mutable so composite paths that only hold `&self` can still stamp usage.
#[derive(Default)]
pub struct TileBudget {
    state: Mutex<BudgetState>,
}

/// Viewport heights rasterized above and below the viewport, so scrolling reveals ready tiles.
pub const RASTER_MARGIN_VIEWPORTS: f64 = 1.0;

/// Viewport heights of slack before the window is re-rastered. Smaller than
/// [`RASTER_MARGIN_VIEWPORTS`], so the re-raster is scheduled while ready tiles remain ahead.
pub const REFRESH_MARGIN_VIEWPORTS: f64 = 0.5;

/// The rasterized band in page space. It spans the full page width, so horizontal scrolling
/// cannot land on a hole.
pub fn raster_window(scroll_y: f64, vp_h: f64) -> (f64, f64) {
    let margin = RASTER_MARGIN_VIEWPORTS * vp_h;
    (scroll_y - margin, scroll_y + vp_h + margin)
}

/// The band that must be resident before the next frame; escaping it triggers a re-raster.
fn refresh_window(scroll_y: f64, vp_h: f64) -> (f64, f64) {
    let margin = REFRESH_MARGIN_VIEWPORTS * vp_h;
    (scroll_y - margin, scroll_y + vp_h + margin)
}

fn clamp_to_page(window: (f64, f64), page_height: f64) -> (f64, f64) {
    let page_bottom = page_height.max(0.0);
    (window.0.max(0.0), window.1.min(page_bottom).max(0.0))
}

/// Park dirty tiles outside the raster window in [`TileState::Deferred`] so stages 5 and 6 skip
/// them; returns how many were parked. Viewport-pinned layers are exempt: their page positions
/// do not track scroll, so a page-space window says nothing about whether they are on screen.
pub fn defer_tiles_outside_window(tile_list: &mut TileList, scroll_y: f64, vp_h: f64) -> usize {
    let (lo, hi) = raster_window(scroll_y, vp_h);
    let layer_list = std::sync::Arc::clone(&tile_list.layer_list);
    let mut deferred = 0;

    for tile in tile_list.arena.values_mut() {
        if tile.state != TileState::Dirty {
            continue;
        }
        if layer_list.layer_anchor(tile.layer_id) != TileAnchor::Scroll {
            continue;
        }
        if tile.rect.y < hi && tile.rect.y + tile.rect.height > lo {
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

    /// Record the window that is now rasterized; [`Self::needs_rerender`] measures against it.
    pub fn note_rastered_window(&self, scroll_y: f64, vp_h: f64, page_height: f64) {
        let window = clamp_to_page(raster_window(scroll_y, vp_h), page_height);
        self.state.lock().rastered = Some(window);
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
    pub fn needs_rerender(&self, scroll_y: f64, vp_h: f64, page_height: f64) -> bool {
        let (lo, hi) = clamp_to_page(refresh_window(scroll_y, vp_h), page_height);
        let st = self.state.lock();

        if st.evicted.iter().any(|r| r.y < hi && r.y + r.height > lo) {
            return true;
        }

        // Tolerance keeps sub-pixel scroll jitter at a page edge from re-rastering every frame.
        const EPS: f64 = 0.5;
        match st.rastered {
            None => false,
            Some((rlo, rhi)) => lo < rlo - EPS || hi > rhi + EPS,
        }
    }

    /// Evict least-recently-composited tile positions until resident pixel bytes fit
    /// `budget_bytes` (`0` disables enforcement).
    ///
    /// Never evicted: tiles overlapping the [`raster_window`], fixed/sticky tiles
    /// (viewport-pinned regardless of scroll), and GPU-resident tiles
    /// (their memory lives in the backend's texture store, not here). Ties in LRU age evict
    /// the position farthest from the viewport first.
    pub fn enforce(
        &self,
        tiles: &mut Vec<BakedTile>,
        pixel_cache: &mut TilePixelCache,
        scroll_y: f64,
        vp_h: f64,
        budget_bytes: usize,
    ) -> EvictionReport {
        struct PosInfo {
            bytes: usize,
            protected: bool,
            rect: Rect,
        }

        let mut st = self.state.lock();
        let (band_lo, band_hi) = raster_window(scroll_y, vp_h);

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
                let in_band = rect.y < band_hi && rect.y + rect.height > band_lo;
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
        let band_center = scroll_y + vp_h / 2.0;
        let mut candidates: Vec<(u64, f64, TilePosKey, usize, Rect)> = positions
            .iter()
            .filter(|(_, p)| !p.protected && p.bytes > 0)
            .map(|(k, p)| {
                let dist = (p.rect.y + p.rect.height / 2.0 - band_center).abs();
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

    fn has_tile_at(tiles: &[BakedTile], y: f64) -> bool {
        tiles.iter().any(|t| t.page_y == y)
    }

    #[test]
    fn budget_zero_disables_eviction() {
        let budget = TileBudget::new();
        let mut tiles = column(8);
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 0);

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

        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 3 * TILE_BYTES);

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

        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 1);

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
        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, TILE_BYTES);

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
        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 10 * TILE_BYTES);
        assert_eq!(report.resident_bytes, 2 * TILE_BYTES);

        // Force eviction of the far tile; its pixel-cache entry must go with it.
        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, TILE_BYTES);
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

        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 1);

        assert_eq!(report.evicted_tiles, 1);
        assert!(cache.is_empty());
    }

    #[test]
    fn fixed_and_gpu_tiles_are_never_evicted() {
        let budget = TileBudget::new();
        let mut fixed = tile(0.0, 4000.0, 1);
        fixed.anchor = TileAnchor::Fixed;
        let gpu = BakedTile {
            pixels: TilePixels::Gpu(7),
            ..tile(0.0, 4256.0, 2)
        };
        let mut tiles = vec![fixed, gpu];
        let mut cache = TilePixelCache::new();

        let report = budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 1);

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
        budget.note_rastered_window(PAGE / 2.0, PAGE, PAGE);
        budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 2 * TILE_BYTES);
        assert!(!has_tile_at(&tiles, 4000.0));

        assert!(!budget.needs_rerender(0.0, TILE, PAGE), "evicted region is far away");
        assert!(
            budget.needs_rerender(3900.0, TILE, PAGE),
            "refresh window reaches the evicted region"
        );

        budget.note_full_raster();
        assert!(
            !budget.needs_rerender(3900.0, TILE, PAGE),
            "full re-raster restores everything"
        );
    }

    #[test]
    fn needs_rerender_follows_the_rastered_window() {
        const PAGE: f64 = 10_000.0;
        let budget = TileBudget::new();

        // Nothing rastered yet: the caller is about to render anyway, so don't ask for another.
        assert!(!budget.needs_rerender(0.0, TILE, PAGE));

        // Rastered around the top: window [0, 512]; the refresh window at scroll 0 is [0, 384].
        budget.note_rastered_window(0.0, TILE, PAGE);
        assert!(!budget.needs_rerender(0.0, TILE, PAGE), "fresh window needs nothing");
        assert!(
            !budget.needs_rerender(100.0, TILE, PAGE),
            "small scroll stays inside the slack"
        );
        assert!(
            budget.needs_rerender(300.0, TILE, PAGE),
            "scrolling past the slack must extend the window"
        );

        // Extending around the new position satisfies the demand again.
        budget.note_rastered_window(300.0, TILE, PAGE);
        assert!(!budget.needs_rerender(300.0, TILE, PAGE));
    }

    #[test]
    fn page_edges_do_not_re_raster_forever() {
        const PAGE: f64 = 1000.0;
        let budget = TileBudget::new();

        // At either page edge the windows run past the page; clamping both to the page is what
        // stops a permanent "needs re-raster" there.
        let bottom = PAGE - TILE;
        budget.note_rastered_window(bottom, TILE, PAGE);
        assert!(!budget.needs_rerender(bottom, TILE, PAGE));

        budget.note_rastered_window(0.0, TILE, PAGE);
        assert!(!budget.needs_rerender(0.0, TILE, PAGE));
    }

    /// Tiles a real 6 000 px layout, so the deferral policy is exercised against the actual
    /// `TileList` the pipeline builds rather than a hand-made stand-in.
    mod deferral {
        use super::*;
        use crate::common::document::pipeline_doc::GosubDocumentAdapter;
        use crate::common::geo::Dimension;
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

        fn tall_page_tiles() -> TileList {
            let mut doc = html_compile::<Config>(
                r#"<html><body style="margin:0"><div style="height:6000px;background:#ccc"></div></body></html>"#,
            );
            doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());

            let mut render_tree = RenderTree::new(Arc::new(GosubDocumentAdapter::<Config>::new(Arc::new(doc))));
            let _ = render_tree.parse();

            let layout_tree = TaffyLayouter::new().layout(render_tree, Some(Dimension::new(512.0, TILE)), 1.0);
            let mut tile_list = TileList::new(LayerList::new(Arc::new(layout_tree)), Dimension::new(TILE, TILE));
            tile_list.generate();
            tile_list
        }

        fn count(tile_list: &TileList, state: TileState) -> usize {
            tile_list.arena.values().filter(|t| t.state == state).count()
        }

        #[test]
        fn defers_tiles_outside_the_window_only() {
            let mut tile_list = tall_page_tiles();
            let total = tile_list.arena.len();
            assert!(total > 20, "a 6 000 px page must produce many tiles, got {total}");

            let deferred = defer_tiles_outside_window(&mut tile_list, 0.0, TILE);
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
            defer_tiles_outside_window(&mut tile_list, 3000.0, TILE);

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
    }
}
