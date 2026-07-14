use crate::common::media::MediaStore;
use crate::common::texture::TextureId;
use crate::common::texture_store::TextureStore;
use crate::tiler::Tile;
use gosub_interface::font_system::FontSystem;
use parking_lot::Mutex;
use std::any::Any;
use std::sync::Arc;

// `RasterStrategy` lives in `gosub_interface` (it is named by the `RenderBackend` trait);
// re-exported here so existing `gosub_render_pipeline::rasterizer::RasterStrategy` paths work.
pub use gosub_interface::render::backend::RasterStrategy;

pub trait Rasterable {
    fn rasterize(&self, tile: &Tile, texture_store: &mut TextureStore, media_store: &MediaStore) -> Option<TextureId>;

    /// The font system this rasterizer draws with, exposed as `dyn FontSystem` so the
    /// layout engine can share the same instance — text is then measured and drawn against
    /// one font collection (consistent metrics, fonts loaded once).
    ///
    /// Returns `None` for rasterizers that don't shape through a [`FontSystem`] (the null
    /// rasterizer, or the Pango/Cairo path); the layouter then uses its own instance.
    fn font_system(&self) -> Option<Arc<Mutex<dyn FontSystem>>> {
        None
    }
}

/// No-op rasterizer for backends that don't rasterize tiles (e.g. the null backend).
pub struct NullRasterizer;

impl Rasterable for NullRasterizer {
    fn rasterize(
        &self,
        _tile: &Tile,
        _texture_store: &mut TextureStore,
        _media_store: &MediaStore,
    ) -> Option<TextureId> {
        None
    }
}

/// Type-erase a backend's rasterizer for return from `RenderBackend::create_rasterizer`.
///
/// `Rasterable` references pipeline-internal types (`Tile`, `TextureStore`, `MediaStore`) that
/// cannot live in `gosub_interface`, so the trait method returns `Box<dyn Any>`. Backends box
/// their `Box<dyn Rasterable>` through this helper; the engine recovers it with
/// [`downcast_rasterizer`].
pub fn erase_rasterizer(rasterizer: Box<dyn Rasterable + Send + Sync>) -> Box<dyn Any + Send + Sync> {
    Box::new(rasterizer)
}

/// Recover a `Box<dyn Rasterable>` from the type-erased value produced by
/// [`erase_rasterizer`] (via `RenderBackend::create_rasterizer`). Returns `None` for backends
/// that don't supply a real rasterizer (the default no-op marker).
pub fn downcast_rasterizer(erased: Box<dyn Any + Send + Sync>) -> Option<Box<dyn Rasterable + Send + Sync>> {
    erased.downcast::<Box<dyn Rasterable + Send + Sync>>().ok().map(|b| *b)
}


// ---------------------------------------------------------------------------
// Baked tiles + stage-6 rasterization strategies
//
// Moved here from the engine's BrowsingContext: these are the per-strategy
// implementations behind `RasterStrategy` (parallel+cached for CPU backends,
// sequential for GPU tile mode), plus the `BakedTile` output type and the
// content-hash pixel cache they share. The engine picks the strategy and
// owns the caches; the rasterization itself lives here with its domain.
// ---------------------------------------------------------------------------

use crate::common::texture::TilePixels;
use crate::render::backend::CachedTile;

/// A single rasterized tile with its page-coordinate position, ready to blit.
pub struct BakedTile {
    pub page_x: f64,
    pub page_y: f64,
    /// Owning layer id. Needed to disambiguate tiles from different layers that share the same
    /// page position — e.g. the base layer and a `position: sticky` header both have a tile at
    /// the top-left `(0,0)`. Keying carry-over by position alone collapses them (see the
    /// engine's hover-repaint carry-over logic).
    pub layer_id: u64,
    pub width: u32,
    pub height: u32,
    /// Tile pixels — CPU bytes (Cairo/Skia) or an opaque GPU texture id (Vello/wgpu).
    pub pixels: TilePixels,
    /// In-memory byte order of the pixels (CPU variant), set by the rasterizer that produced it.
    pub format: crate::render::backend::PixelFormat,
    /// Group opacity (1.0 = opaque) of this tile's layer, applied by the compositor.
    pub opacity: f32,
    /// How this tile's layer responds to scroll (normal flow vs. `position: fixed`).
    pub anchor: crate::render::backend::TileAnchor,
}

/// Key that uniquely identifies a tile's content for cache lookup.
/// Format: (page_x bits, page_y bits, layer_id, paint-command hash).
pub type TileCacheKey = (u64, u64, u64, u64);

/// Rasterized tile cache: maps a [`TileCacheKey`] to `(physical_width, physical_height, pixels)`.
/// Carried between renders so unchanged tiles skip rasterization.
pub type TilePixelCache = std::collections::HashMap<TileCacheKey, (u32, u32, TilePixels)>;

/// Compute a stable cache key for a tile: (page_x bits, page_y bits, layer_id, content hash).
/// The content hash covers all paint commands so any visual change produces a different key.
pub fn tile_cache_key(tile: &crate::tiler::Tile) -> TileCacheKey {
    use crate::painter::commands::{
        border::{BorderRadius, BorderStyle},
        brush::Brush,
        gradient::Gradient,
        PaintCommand,
    };

    // Minimal inline FNV-1a hasher — no trait bounds needed on the types being hashed.
    let mut h: u64 = 14695981039346656037;
    macro_rules! fnv {
        ($bytes:expr) => {
            for b in $bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(1099511628211);
            }
        };
    }
    macro_rules! hf32 {
        ($v:expr) => {
            fnv!(&$v.to_bits().to_le_bytes())
        };
    }
    macro_rules! hf64 {
        ($v:expr) => {
            fnv!(&$v.to_bits().to_le_bytes())
        };
    }
    macro_rules! hu64 {
        ($v:expr) => {
            fnv!(&($v as u64).to_le_bytes())
        };
    }
    macro_rules! hbool {
        ($v:expr) => {
            fnv!(&[$v as u8])
        };
    }
    macro_rules! hstr {
        ($s:expr) => {
            fnv!($s.as_bytes());
            fnv!(&[0u8])
        };
    }

    macro_rules! hash_brush {
        ($b:expr) => {
            match $b {
                Brush::Solid(c) => {
                    fnv!(&[0]);
                    hf32!(c.r());
                    hf32!(c.g());
                    hf32!(c.b());
                    hf32!(c.a());
                }
                Brush::Image(m) => {
                    fnv!(&[1]);
                    hu64!(m.as_u64());
                }
                Brush::Gradient(Gradient::Linear(g)) => {
                    fnv!(&[2]);
                    hf32!(g.angle_deg);
                    for stop in &g.stops {
                        hf32!(stop.offset);
                        hf32!(stop.color.r());
                        hf32!(stop.color.g());
                        hf32!(stop.color.b());
                        hf32!(stop.color.a());
                    }
                }
            }
        };
    }

    // Hash tile background.
    match tile.bgcolor {
        Some((r, g, b, a)) => {
            hbool!(true);
            hf32!(r);
            hf32!(g);
            hf32!(b);
            hf32!(a);
        }
        None => hbool!(false),
    }

    for elem in &tile.elements {
        hu64!(elem.id.as_u64());
        hf64!(elem.rect.x);
        hf64!(elem.rect.y);
        hf64!(elem.rect.width);
        hf64!(elem.rect.height);

        for cmd in &elem.paint_commands {
            match cmd {
                // Scene-only layer-group markers; never present in per-tile commands, so they don't
                // affect a tile's content hash.
                PaintCommand::PushLayer { .. } | PaintCommand::PopLayer => {}
                PaintCommand::Rectangle(r) => {
                    fnv!(&[0u8]);
                    let rect = r.rect();
                    hf64!(rect.x);
                    hf64!(rect.y);
                    hf64!(rect.width);
                    hf64!(rect.height);
                    fnv!(&[r.blend_mode().id()]);
                    match r.background() {
                        None => hbool!(false),
                        Some(b) => {
                            hbool!(true);
                            hash_brush!(b);
                        }
                    }
                    let border = r.border();
                    hf32!(border.width());
                    fnv!(&[match border.style() {
                        BorderStyle::Solid => 1,
                        BorderStyle::Dashed => 2,
                        BorderStyle::Dotted => 3,
                        BorderStyle::Double => 4,
                        BorderStyle::Groove => 5,
                        BorderStyle::Ridge => 6,
                        BorderStyle::Inset => 7,
                        BorderStyle::Outset => 8,
                        BorderStyle::Hidden => 9,
                        BorderStyle::None => 0,
                    }]);
                    for b in border.brushes() {
                        hash_brush!(&b);
                    }
                    if let Some(tr) = border.radius() {
                        hbool!(true);
                        for br in [&tr.top, &tr.right, &tr.bottom, &tr.left] {
                            match br {
                                BorderRadius::Uniform(v) => {
                                    fnv!(&[0]);
                                    hf32!(*v);
                                }
                                BorderRadius::Elliptical { horizontal, vertical } => {
                                    fnv!(&[1]);
                                    hf32!(*horizontal);
                                    hf32!(*vertical);
                                }
                            }
                        }
                    } else {
                        hbool!(false);
                    }
                    let (tl, tr, br, bl) = r.radius_x();
                    hf64!(tl);
                    hf64!(tr);
                    hf64!(br);
                    hf64!(bl);
                    let (tl, tr, br, bl) = r.radius_y();
                    hf64!(tl);
                    hf64!(tr);
                    hf64!(br);
                    hf64!(bl);
                }
                PaintCommand::Text(t) => {
                    fnv!(&[1u8]);
                    hf64!(t.rect.x);
                    hf64!(t.rect.y);
                    hf64!(t.rect.width);
                    hf64!(t.rect.height);
                    hstr!(&t.text);
                    hstr!(&t.font_info.family);
                    hf64!(t.font_info.size);
                    hf64!(t.font_info.line_height);
                    hu64!(t.font_info.weight as u64);
                    hu64!(t.font_info.width as u64);
                    hu64!(t.font_info.slant as u64);
                    hbool!(t.font_info.underline);
                    hbool!(t.font_info.line_through);
                    hash_brush!(&t.brush);
                }
                PaintCommand::Svg(s) => {
                    fnv!(&[2u8]);
                    hu64!(s.media_id.as_u64());
                    let rect = s.rect.rect();
                    hf64!(rect.x);
                    hf64!(rect.y);
                    hf64!(rect.width);
                    hf64!(rect.height);
                }
            }
        }
    }

    (tile.rect.x.to_bits(), tile.rect.y.to_bits(), tile.layer_id.as_u64(), h)
}
/// Sequential per-tile rasterization, used by GPU backends (e.g. Vello) whose shared
/// `Mutex<Renderer>` rules out parallelism. No dirty-tile cache, so it returns an empty one.
pub fn rasterize_sequential(
    rasterizer: &(dyn Rasterable + Send + Sync),
    layer_ids: &[crate::layering::layer::LayerId],
    tile_list: &mut crate::tiler::TileList,
    full_page_rect: crate::common::geo::Rect,
    media_store: &crate::common::media::MediaStore,
) -> (Vec<BakedTile>, TilePixelCache) {
    use crate::common::texture_store::TextureStore;
    use crate::tiler::TileState;
    use gosub_shared::{timing_start, timing_stop};

    let ts6 = timing_start!("pipeline.rasterize");
    let mut texture_store = TextureStore::new();

    for &layer_id in layer_ids {
        let tile_ids = tile_list.get_intersecting_tiles(layer_id, full_page_rect);
        for tile_id in tile_ids {
            if let Some(tile) = tile_list.get_tile_mut(tile_id) {
                if tile.state == TileState::Dirty {
                    match rasterizer.rasterize(tile, &mut texture_store, media_store) {
                        Some(texture_id) => {
                            tile.texture_id = Some(texture_id);
                            tile.state = TileState::Clean;
                        }
                        None => tile.state = TileState::Empty,
                    }
                }
            }
        }
    }

    let mut tiles: Vec<BakedTile> = Vec::with_capacity(tile_list.arena.len());
    for tile in tile_list.arena.values() {
        if let (Some(texture_id), true) = (tile.texture_id, tile.state == TileState::Clean) {
            if let Some(tex) = texture_store.get(texture_id) {
                tiles.push(BakedTile {
                    page_x: tile.rect.x,
                    page_y: tile.rect.y,
                    layer_id: tile.layer_id.as_u64(),
                    width: tex.width as u32,
                    height: tex.height as u32,
                    pixels: tex.pixels.clone(),
                    format: tex.format,
                    opacity: tile_list.layer_list.layer_opacity(tile.layer_id),
                    anchor: tile_list.layer_list.layer_anchor(tile.layer_id),
                });
            }
        }
    }

    timing_stop!(ts6);
    (tiles, std::collections::HashMap::new())
}

/// Parallel per-tile rasterization with the dirty-tile pixel cache, used by CPU backends
/// (Cairo, Skia) whose rasterizers are `Send + Sync`. For each dirty tile: if its content
/// hash matches an entry in `prev_tile_cache`, the previous render's pixels are reused;
/// otherwise the tile is rasterized on a rayon worker. Returns the baked tiles plus the
/// new pixel cache for the next render.
pub fn rasterize_parallel(
    rasterizer: &(dyn Rasterable + Send + Sync),
    layer_ids: &[crate::layering::layer::LayerId],
    tile_list: &mut crate::tiler::TileList,
    full_page_rect: crate::common::geo::Rect,
    media_store: &crate::common::media::MediaStore,
    prev_tile_cache: &TilePixelCache,
    timing_label: &str,
) -> (Vec<BakedTile>, TilePixelCache) {
    use crate::common::texture_store::TextureStore;
    use crate::render::backend::PixelFormat;
    use crate::tiler::{TileId, TileState};
    use gosub_shared::{timing_start, timing_stop};
    use rayon::prelude::*;

    // Cairo and Skia both emit premultiplied ARGB32 (BGRA byte order).
    let tile_format = PixelFormat::PreMulArgb32;

    let ts6 = timing_start!(timing_label);

    // Phase 1: collect IDs of dirty tiles across all layers.
    let dirty_ids: Vec<TileId> = layer_ids
        .iter()
        .flat_map(|&layer_id| tile_list.get_intersecting_tiles(layer_id, full_page_rect))
        .filter(|&id| tile_list.arena.get(&id).is_some_and(|t| t.state == TileState::Dirty))
        .collect();

    // Phase 2: parallel rasterization with dirty-tile cache.
    // For each tile: compute a content hash; if it matches the previous render's cached
    // pixels, reuse them (cache hit). Otherwise rasterize on this thread.
    // Result: (tile_id, Option<BakedTile>, Option<new_cache_entry>)
    type CacheEntry = (TileCacheKey, (u32, u32, TilePixels));
    let results: Vec<(TileId, Option<BakedTile>, Option<CacheEntry>)> = dirty_ids
        .par_iter()
        .map(|&tile_id| {
            let Some(tile) = tile_list.arena.get(&tile_id) else {
                return (tile_id, None, None);
            };

            let key = tile_cache_key(tile);

            // Cache hit: same content as the previous render — reuse pixels.
            if let Some(&(w, h, ref data)) = prev_tile_cache.get(&key) {
                let baked = BakedTile {
                    page_x: tile.rect.x,
                    page_y: tile.rect.y,
                    layer_id: tile.layer_id.as_u64(),
                    width: w,
                    height: h,
                    pixels: data.clone(),
                    format: tile_format,
                    opacity: tile_list.layer_list.layer_opacity(tile.layer_id),
                    anchor: tile_list.layer_list.layer_anchor(tile.layer_id),
                };
                return (tile_id, Some(baked), None);
            }

            // Cache miss: rasterize and emit a new cache entry.
            let mut local_store = TextureStore::new();
            let baked = rasterizer
                .rasterize(tile, &mut local_store, media_store)
                .and_then(|tid| local_store.get(tid))
                .map(|tex| BakedTile {
                    page_x: tile.rect.x,
                    page_y: tile.rect.y,
                    layer_id: tile.layer_id.as_u64(),
                    width: tex.width as u32,
                    height: tex.height as u32,
                    pixels: tex.pixels.clone(),
                    format: tex.format,
                    opacity: tile_list.layer_list.layer_opacity(tile.layer_id),
                    anchor: tile_list.layer_list.layer_anchor(tile.layer_id),
                });

            let cache_entry = baked.as_ref().map(|b| (key, (b.width, b.height, b.pixels.clone())));
            (tile_id, baked, cache_entry)
        })
        .collect();

    // Phase 3: update tile states, gather BakedTiles, and build the new tile cache.
    let mut tiles: Vec<BakedTile> = Vec::with_capacity(results.len());
    let mut new_tile_cache: TilePixelCache = std::collections::HashMap::with_capacity(results.len());

    for (tile_id, baked, cache_entry) in results {
        if let Some(tile) = tile_list.arena.get_mut(&tile_id) {
            match baked {
                Some(b) => {
                    tile.state = TileState::Clean;
                    if let Some(entry) = cache_entry {
                        new_tile_cache.insert(entry.0, entry.1);
                    }
                    tiles.push(b);
                }
                None => {
                    tile.state = TileState::Empty;
                }
            }
        }
    }

    timing_stop!(ts6);
    (tiles, new_tile_cache)
}
/// Build the CPU `CachedTile` list for the zero-copy scroll handle. GPU-resident tiles have no
/// CPU pixel buffer (they're composited by the backend), so they're skipped here.
pub fn cpu_cached_tiles(baked: &[BakedTile]) -> Vec<CachedTile> {
    baked
        .iter()
        .filter_map(|t| match &t.pixels {
            TilePixels::Cpu(d) => Some(CachedTile {
                page_x: t.page_x as f32,
                page_y: t.page_y as f32,
                width: t.width,
                height: t.height,
                data: d.clone(),
                format: t.format,
                opacity: t.opacity,
                anchor: t.anchor,
                // Alpha is the 4th byte in both supported formats ([B,G,R,A] / [R,G,B,A]). Scanned
                // once here (per cache build, not per scroll) so the compositor can fast-path it.
                opaque: d.chunks_exact(4).all(|px| px[3] == 0xFF),
            }),
            TilePixels::Gpu(_) => None,
        })
        .collect()
}

/// Placed GPU tiles for the current page, in page coordinates — handed to a GPU backend's
/// `composite_tiles` step. Empty for CPU backends.
pub fn collect_placed_gpu_tiles(baked: &[BakedTile]) -> Vec<crate::render::backend::PlacedGpuTile> {
    baked
        .iter()
        .filter_map(|t| {
            if let TilePixels::Gpu(id) = t.pixels {
                Some(crate::render::backend::PlacedGpuTile {
                    page_x: t.page_x as f32,
                    page_y: t.page_y as f32,
                    width: t.width,
                    height: t.height,
                    texture_id: id,
                    opacity: t.opacity,
                    anchor: t.anchor,
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_downcast_roundtrip_recovers_rasterizer() {
        let erased = erase_rasterizer(Box::new(NullRasterizer));
        assert!(downcast_rasterizer(erased).is_some());
    }

    #[test]
    fn downcast_of_non_rasterizer_marker_is_none() {
        // The default `RenderBackend::create_rasterizer` returns `Box::new(())`; the engine
        // must treat that as "no rasterizer" rather than panicking.
        let marker: Box<dyn Any + Send + Sync> = Box::new(());
        assert!(downcast_rasterizer(marker).is_none());
    }
}
