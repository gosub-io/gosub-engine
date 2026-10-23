//! Tiles drawn at the scale a zoomed-in page is shown at.
//!
//! The page's own tiles are rasterized at the device pixel ratio, over a band of the layout
//! viewport. Zoomed in further than that they are stretched, and text goes soft. So the part of
//! the page on screen gets a second set of tiles, rasterized at the zoom: the same elements,
//! taken from the page grid's element lists and painted again, on a grid of their own whose tiles
//! are a fixed size in texture pixels however far the page is zoomed. Only the tiles around the
//! visual viewport are kept.
//!
//! A layer these tiles cover on screen is composited from them, any other from the page's tiles,
//! so a layer is never drawn twice and never left with a hole.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gosub_render_pipeline::common::browser_state::BrowserState;
use gosub_render_pipeline::common::geo::{Coordinate, Rect as PipelineRect};
use gosub_render_pipeline::common::media::MediaStore;
use gosub_render_pipeline::common::texture::TilePixels;
use gosub_render_pipeline::common::texture_store::TextureStore;
use gosub_render_pipeline::layering::layer::LayerId;
use gosub_render_pipeline::painter::Painter;
use gosub_render_pipeline::rasterizer::Rasterable;
use gosub_render_pipeline::render::backend::{anchored_tile_pos, CompositeView, PlacedGpuTile};
use gosub_render_pipeline::tiler::{Tile, TileId, TileList, TileState, TiledLayoutElement};

/// The side of a zoom tile, in texture pixels. Small enough that the few around the visual
/// viewport stay a few MB each at any zoom, large enough that a phone screen needs a dozen.
pub(crate) const ZOOM_TILE_PX: f64 = 512.0;

/// (layer, column, row) on the zoom grid.
type Key = (u64, i64, i64);

/// What rasterizing one cell gave.
enum Cell {
    /// Nothing paints there.
    Empty,
    Drawn(TilePixels),
    /// The rasterizer gave nothing back: left out, so the page's tiles show there and the next
    /// fill tries again.
    Failed,
}

/// The zoom tiles of one scale, for the page content of one pipeline pass.
pub(crate) struct ZoomTiles {
    /// Texture pixels per CSS px.
    scale: f64,
    /// The side of a tile, CSS px.
    side: f64,
    /// The page content these were painted from; see `BrowsingContext::content_generation`.
    generation: u64,
    /// `None` where nothing paints.
    tiles: HashMap<Key, Option<TilePixels>>,
}

impl ZoomTiles {
    pub(crate) fn new(scale: f64, generation: u64) -> Self {
        Self {
            scale,
            side: ZOOM_TILE_PX / scale,
            generation,
            tiles: HashMap::new(),
        }
    }

    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The grid cells over `rect`, in layer coordinates.
    fn cells(&self, layer: u64, rect: PipelineRect) -> impl Iterator<Item = Key> {
        let side = self.side;
        let col = |v: f64| (v / side).floor() as i64;
        // A rect that ends on a cell edge does not reach into the next cell; less than half a
        // texture pixel of one does not show either.
        let inset = (0.5 / self.scale).min(rect.width / 2.0).min(rect.height / 2.0);
        let (c0, c1) = (col(rect.x), col(rect.x + rect.width - inset));
        let (r0, r1) = (col(rect.y), col(rect.y + rect.height - inset));
        (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (layer, c, r)))
    }

    fn cell_rect(&self, (_, c, r): Key) -> PipelineRect {
        PipelineRect::new(c as f64 * self.side, r as f64 * self.side, self.side, self.side)
    }

    /// Whether every tile `rect` of `layer` needs is here.
    pub(crate) fn covers(&self, layer: LayerId, rect: PipelineRect) -> bool {
        self.cells(layer.as_u64(), rect).all(|k| self.tiles.contains_key(&k))
    }

    /// The tiles over `rect` of `layer`, to composite.
    pub(crate) fn placed(&self, layer: LayerId, rect: PipelineRect, tile_list: &TileList) -> Vec<PlacedGpuTile> {
        let opacity = tile_list.layer_list.layer_opacity(layer);
        let anchor = tile_list.layer_list.layer_anchor(layer);
        self.cells(layer.as_u64(), rect)
            .filter_map(|k| match self.tiles.get(&k) {
                Some(Some(TilePixels::Gpu(tile))) => {
                    let cell = self.cell_rect(k);
                    Some(PlacedGpuTile {
                        page_x: cell.x as f32,
                        page_y: cell.y as f32,
                        width: cell.width as f32,
                        height: cell.height as f32,
                        texture_id: tile.id(),
                        opacity,
                        anchor,
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// Rasterize every tile the view shows that is not here yet, and let go of those well away
    /// from it. Returns whether any tile was added or dropped.
    pub(crate) fn fill(
        &mut self,
        view: &CompositeView,
        tile_list: &TileList,
        rasterizer: &(dyn Rasterable + Send + Sync),
        media_store: &MediaStore,
        paint_state: &BrowserState,
    ) -> bool {
        let layer_ids = tile_list.layer_list.layer_ids.read().clone();
        let mut painter: Option<Painter> = None;
        let mut changed = false;
        for &layer in &layer_ids {
            let rect = layer_rect(view, tile_list, layer);
            let missing: Vec<Key> = self
                .cells(layer.as_u64(), rect)
                .filter(|k| !self.tiles.contains_key(k))
                .collect();
            for key in missing {
                let painter = painter.get_or_insert_with(|| {
                    Painter::new(Arc::clone(&tile_list.layer_list), rasterizer.font_system())
                        .with_shape_cache(Arc::clone(&tile_list.shape_cache))
                });
                let pixels = match self.rasterize(key, layer, tile_list, painter, paint_state, rasterizer, media_store)
                {
                    Cell::Empty => None,
                    Cell::Drawn(pixels) => Some(pixels),
                    Cell::Failed => continue,
                };
                self.tiles.insert(key, pixels);
                changed = true;
            }
        }

        // A tile within half a screen of what is shown stays, so panning back and forth does not
        // rasterize the same tiles over and over, and a long pan does not pile them up.
        let keep: HashMap<u64, PipelineRect> = layer_ids
            .iter()
            .map(|&layer| {
                let r = layer_rect(view, tile_list, layer);
                let grown = PipelineRect::new(r.x - r.width / 2.0, r.y - r.height / 2.0, r.width * 2.0, r.height * 2.0);
                (layer.as_u64(), grown)
            })
            .collect();
        let before = self.tiles.len();
        let side = self.side;
        self.tiles
            .retain(|&(layer, c, r), _| keep.get(&layer).is_some_and(|k| overlaps(c, r, side, k)));
        changed || self.tiles.len() != before
    }

    /// Let go of the tiles over `rect`, in every layer: what they show was painted again.
    pub(crate) fn forget(&mut self, rect: PipelineRect) {
        let side = self.side;
        self.tiles.retain(|&(_, c, r), _| !overlaps(c, r, side, &rect));
    }

    /// Let go of the tiles that reach outside rows `lo..hi`, in every layer: the page grid they
    /// were painted from ended there, so they miss whatever lies beyond it.
    pub(crate) fn forget_outside_rows(&mut self, lo: f64, hi: f64) {
        let side = self.side;
        self.tiles
            .retain(|&(_, _, r), _| r as f64 * side >= lo && (r + 1) as f64 * side <= hi);
    }

    #[allow(clippy::too_many_arguments)]
    fn rasterize(
        &self,
        key: Key,
        layer: LayerId,
        tile_list: &TileList,
        painter: &Painter,
        paint_state: &BrowserState,
        rasterizer: &(dyn Rasterable + Send + Sync),
        media_store: &MediaStore,
    ) -> Cell {
        let rect = self.cell_rect(key);
        // The page grid has already worked out which elements paint where; any element that
        // reaches this cell is in a page tile that does.
        let near: HashSet<_> = tile_list
            .get_intersecting_tiles(layer, rect)
            .into_iter()
            .filter_map(|id| tile_list.arena.get(&id))
            .flat_map(|tile| tile.elements.iter().map(|e| e.id))
            .collect();
        if near.is_empty() {
            return Cell::Empty;
        }
        // In the layer's paint order, which the page tiles keep but their union does not.
        let elements: Vec<TiledLayoutElement> = {
            let layers = tile_list.layer_list.layers.read();
            let Some(layer) = layers.get(&layer) else {
                return Cell::Failed;
            };
            layer
                .elements
                .iter()
                .filter(|id| near.contains(id))
                .map(|&id| {
                    let mut element = TiledLayoutElement {
                        id,
                        rect: PipelineRect::new(0.0, 0.0, rect.width, rect.height),
                        position: Coordinate::new(0.0, 0.0),
                        paint_commands: Vec::new(),
                    };
                    element.paint_commands = painter.paint(&element, paint_state);
                    element
                })
                .collect()
        };
        let tile = Tile {
            id: TileId::new(0),
            layer_id: layer,
            elements,
            texture_id: None,
            state: TileState::Dirty,
            rect,
            bgcolor: None,
        };
        let mut store = TextureStore::new();
        let pixels = rasterizer
            .rasterize_at(&tile, self.scale, &mut store, media_store)
            .and_then(|id| store.get(id).map(|t| t.pixels.clone()));
        pixels.map_or(Cell::Failed, Cell::Drawn)
    }
}

/// Whether cell `(c, r)` of a grid of `side` reaches into `rect`.
fn overlaps(c: i64, r: i64, side: f64, rect: &PipelineRect) -> bool {
    let (x, y) = (c as f64 * side, r as f64 * side);
    x < rect.x + rect.width && x + side > rect.x && y < rect.y + rect.height && y + side > rect.y
}

/// The part of `layer` the view shows, in the layer's own coordinates: the visual viewport, moved
/// the way the compositor moves that layer's tiles.
pub(crate) fn layer_rect(view: &CompositeView, tile_list: &TileList, layer: LayerId) -> PipelineRect {
    let anchor = tile_list.layer_list.layer_anchor(layer);
    let (dx, dy) = anchored_tile_pos(0.0, 0.0, view.scroll.0, view.scroll.1, anchor);
    PipelineRect::new(view.origin.0 - dx, view.origin.1 - dy, view.size.0, view.size.1)
}
