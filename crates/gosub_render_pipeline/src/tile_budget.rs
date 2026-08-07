//! Tile-cache memory budget with LRU eviction.
//!
//! The pipeline rasterizes every tile of the full page eagerly, so without a bound a very tall
//! page holds hundreds of megabytes of pixels. [`TileBudget`] caps the bytes held by a page's
//! [`BakedTile`]s and the cross-render [`TilePixelCache`], evicting least-recently-composited
//! tiles first while never touching the near-viewport band.
//!
//! Eviction only removes pixels and records where they were - restoring an evicted tile is the
//! caller's decision (per-tile re-rasterization is not always possible, e.g. remotely rendered
//! caches). The caller polls [`TileBudget::needs_rerender`] on scroll and schedules a full
//! re-render when an evicted region approaches the viewport.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

use crate::common::geo::Rect;
use crate::common::texture::TilePixels;
use crate::rasterizer::{BakedTile, TilePixelCache};
use crate::render::backend::{anchored_tile_pos, TileAnchor};

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
}

/// LRU bookkeeping and budget enforcement for one browsing context's tile caches.
/// Interior-mutable so composite paths that only hold `&self` can still stamp usage.
#[derive(Default)]
pub struct TileBudget {
    state: Mutex<BudgetState>,
}

/// The never-evict band in page space: the viewport plus one viewport height above and below.
/// The band spans the full page width so horizontal scrolling cannot land on a hole.
fn protection_band(scroll_y: f64, vp_h: f64) -> (f64, f64) {
    (scroll_y - vp_h, scroll_y + 2.0 * vp_h)
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
    }

    /// Every tile was just re-rasterized, so previously evicted regions are live again.
    pub fn note_full_raster(&self) {
        self.state.lock().evicted.clear();
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

    /// True when the near-viewport band at the given scroll position overlaps an evicted region.
    /// Compositing alone cannot restore evicted pixels; the caller must schedule a re-render.
    pub fn needs_rerender(&self, scroll_y: f64, vp_h: f64) -> bool {
        let (lo, hi) = protection_band(scroll_y, vp_h);
        self.state
            .lock()
            .evicted
            .iter()
            .any(|r| r.y < hi && r.y + r.height > lo)
    }

    /// Evict least-recently-composited tile positions until resident pixel bytes fit
    /// `budget_bytes` (`0` disables enforcement).
    ///
    /// Never evicted: tiles overlapping the protection band (viewport ± one viewport height),
    /// fixed/sticky tiles (viewport-pinned regardless of scroll), and GPU-resident tiles
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
        let (band_lo, band_hi) = protection_band(scroll_y, vp_h);

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
        let budget = TileBudget::new();
        let mut tiles = column(2);
        tiles.push(tile(0.0, 4000.0, 0));
        let mut cache = TilePixelCache::new();

        budget.enforce(&mut tiles, &mut cache, 0.0, TILE, 2 * TILE_BYTES);
        assert!(!has_tile_at(&tiles, 4000.0));

        assert!(!budget.needs_rerender(0.0, TILE), "evicted region is far away");
        assert!(budget.needs_rerender(3800.0, TILE), "band reaches the evicted region");

        budget.note_full_raster();
        assert!(
            !budget.needs_rerender(3800.0, TILE),
            "full re-raster restores everything"
        );
    }
}
