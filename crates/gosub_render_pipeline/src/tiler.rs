use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::geo::{Coordinate, Dimension, Rect};
use crate::common::texture::TextureId;
use crate::layering::layer::{LayerId, LayerList};
use crate::layouter::{LayoutElementId, LayoutElementNode};
use crate::painter::commands::shadow::blur_reach;
use crate::painter::commands::PaintCommand;
use crate::painter::shape_cache::ShapeCache;
use crate::render::backend::TileAnchor;
use parking_lot::RwLock;
use rstar::primitives::GeomWithData;
use rstar::AABB;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::ops::AddAssign;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(u64);

/// Tile-coverage extent of an element's paint: the union of margin and border box. Negative
/// margins shrink the margin box below the painted border box (even to zero area), so the
/// margin box alone would drop tiles the element actually paints into.
fn paint_extent(el: &LayoutElementNode) -> Rect {
    let m = el.box_model.margin_box;
    let b = el.box_model.border_box;
    let x = m.x.min(b.x);
    let y = m.y.min(b.y);
    let x2 = (m.x + m.width).max(b.x + b.width);
    let y2 = (m.y + m.height).max(b.y + b.height);
    Rect::new(x, y, x2 - x, y2 - y)
}

impl TileId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

impl AddAssign<u64> for TileId {
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl std::fmt::Display for TileId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TileId({})", self.0)
    }
}

/// The (possibly partial) part of an element that falls within a single tile.
#[derive(Debug, Clone)]
pub struct TiledLayoutElement {
    pub id: LayoutElementId,
    /// Part of the element to draw, in element-local coordinates (a subset when the
    /// element spans multiple tiles). See the diagram below.
    pub rect: Rect,
    /// Where inside the tile the element starts. See the diagram below.
    pub position: Coordinate,
    pub paint_commands: Vec<PaintCommand>,
}

/*

Here is a box element (id 67) centered within 4 tiles. The tiles are 100x50 each.
The rect size is 100x50.

In tile 1, the rect of element 67 is (0, 0, 50, 25). The position is (50, 25)
In tile 2, the rect of element 67 is (50, 0, 50, 25). The position is (0, 25)
In tile 3, the rect of element 67 is (0, 25, 50, 25). The position is (50, 0).
In tile 4, the rect of element 67 is (50, 25, 50, 25). The position is (0, 0).

So in tile 1 the element starts at 50x25; even though the element is 100x50, the rect
covers only 0,0..50,25 - the top left quarter of the element.

    0                 100             200
    +------------------+----------------+
    |                  |                |
    |            ######|######          |
    |            ######|######          |
    |            ######|######          |
 50 +------------------+----------------+
    |            ######|######          |
    |            ######|######          |
    |            ######|######          |
    |                  |                |
100 +------------------+----------------+
*/

#[derive(Clone, Debug, PartialEq)]
pub enum TileState {
    /// Tile texture is clean and can be rendered
    Ready,
    /// Tile texture needs a repaint
    Dirty,
    /// Tile texture cannot be rendered by this backend
    Unrenderable,
    /// Tile is clean, but it does not contain anything (ie: no texture needed)
    Empty,
    /// Outside the raster window: intentionally not painted or rasterized yet. Becomes `Dirty`
    /// when the window moves over it (see `tile_budget::defer_tiles_outside_window`).
    Deferred,
}

/// Single tile: the elements it covers plus the rendered texture that gets composited to screen.
#[derive(Debug, Clone)]
pub struct Tile {
    pub id: TileId,
    pub layer_id: LayerId,
    pub elements: Vec<TiledLayoutElement>,
    /// None means never rendered. When state is Dirty the id is still valid, but stale.
    pub texture_id: Option<TextureId>,
    pub state: TileState,
    /// Position and dimension of the tile in layer coordinates.
    pub rect: Rect,
    /// Background color of the whole canvas, not of this tile. We should deal with this differently.
    pub bgcolor: Option<(f32, f32, f32, f32)>,
}

/// The tiles of one layer that exist right now, by grid cell. A layer only has tiles for the
/// band of the page a pass asked for (see [`TileList::cover`]), so this is sparse.
#[derive(Debug, Clone)]
pub struct TileLayer {
    pub layer_id: LayerId,
    /// Keyed by (row, column); ordered, so a band of rows is a range.
    cells: BTreeMap<(usize, usize), TileId>,
}

/// An element in a [`LayerIndex`]: its paint rect, keyed by its paint order in the layer.
type IndexedElement = GeomWithData<rstar::primitives::Rectangle<[f64; 2]>, (usize, LayoutElementId, Rect)>;

/// What the page looks like to the tiler, built once per grid: the layout cannot change under a
/// `TileList`, so every [`TileList::cover`] after the first only adds and drops rows.
struct GridIndex {
    /// Background color of the whole canvas (see [`Tile::bgcolor`]).
    bgcolor: Option<(f32, f32, f32, f32)>,
    /// In stacking order.
    layers: Vec<LayerIndex>,
}

struct LayerIndex {
    layer_id: LayerId,
    anchor: TileAnchor,
    /// Cells the layer can ever have tiles in: rows `.0...1`, columns `.2...3`. `None` when the
    /// layer paints nothing inside the page box.
    extent: Option<(usize, usize, usize, usize)>,
    /// Each element's paint rect (outline and outer shadow included), with its paint order.
    elements: rstar::RTree<IndexedElement>,
}

/// The part of the page a pass needs tiles for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TileBand {
    /// Every tile of the page. This grows with the page, so it is for tools and tests that
    /// render a page whole, never for a page the user is browsing.
    Page,
    /// The page-space rows `lo..hi`, a band around a viewport `vp_h` tall. Viewport-pinned
    /// layers get the rows that can show inside it instead (see [`TileList::cover`]).
    Rows { lo: f64, hi: f64, vp_h: f64 },
}

/// Main list of tiles per layer.
#[derive(Clone)]
pub struct TileList {
    pub layer_list: Arc<LayerList>,

    pub tiles: HashMap<LayerId, TileLayer>,

    pub arena: HashMap<TileId, Tile>,
    /// Per-layer element index, built on the first [`Self::cover`].
    index: Option<Arc<GridIndex>>,
    /// Tiles painted since the grid was generated or reset. A reused grid has to drop the
    /// commands of the pass before it, and this is what keeps that from being a walk over
    /// every element on the page.
    painted: Vec<TileId>,
    next_node_id: Arc<RwLock<TileId>>,

    pub default_tile_dimension: Dimension,
    /// Text shaped by the passes over this grid. It belongs to the grid because that is exactly
    /// how long a shaped run stays valid: anything that can move a box builds a new grid, and
    /// web fonts are registered before the first layout, so no grid outlives the font set its
    /// runs were shaped against. A scroll that only extends the raster window keeps the grid,
    /// and that is the case the cache exists for.
    pub shape_cache: Arc<ShapeCache>,
}

impl Debug for TileList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileList")
            .field("layers", &self.tiles)
            .field("arena", &self.arena)
            .field("indexed", &self.index.is_some())
            .field("next_node_id", &self.next_node_id)
            .field("default_tile_dimension", &self.default_tile_dimension)
            .field("shape_cache", &self.shape_cache)
            .finish()
    }
}

impl TileList {
    pub fn get_tiles_for_element(&self, element_id: LayoutElementId) -> Vec<TileId> {
        let mut matching_tiles = vec![];

        for tile in self.arena.values() {
            for element in &tile.elements {
                if element.id == element_id {
                    matching_tiles.push(tile.id);
                }
            }
        }

        matching_tiles
    }

    /// Put every tile back into the state [`Self::generate`] leaves it in, keeping the grid.
    ///
    /// The grid, the element-to-tile assignment and the R-tree are pure functions of the layer
    /// list and the tile dimension. A pass that changes neither - a scroll extending the raster
    /// window - would regenerate exactly the same geometry, so it resets the per-pass state
    /// instead. On a 29 000 px article that is 1 201 tiles and 36 827 element-to-tile entries
    /// not built again, most of the tiling cost.
    ///
    /// The paint commands go too. A grid that outlives its pass would otherwise accumulate a
    /// whole page of dead commands, because a tile that is carried over or deferred is never
    /// painted over - but only the tiles [`Self::note_painted`] recorded can be holding any, so
    /// this costs a walk of one window's worth of tiles rather than of every element on the page.
    pub fn reset_states(&mut self) {
        for tile in self.arena.values_mut() {
            tile.state = TileState::Dirty;
            tile.texture_id = None;
        }
        for tile_id in std::mem::take(&mut self.painted) {
            let Some(tile) = self.arena.get_mut(&tile_id) else {
                continue;
            };
            for element in &mut tile.elements {
                element.paint_commands = Vec::new();
            }
        }
    }

    /// Record the tiles a paint pass just wrote commands into, so a later [`Self::reset_states`]
    /// knows which ones to release. Every paint pass must report, or a reused grid keeps
    /// commands it will never paint over.
    pub fn note_painted(&mut self, tile_ids: impl IntoIterator<Item = TileId>) {
        self.painted.extend(tile_ids);
    }

    pub fn invalidate_all(&mut self) {
        for tile in self.arena.values_mut() {
            tile.state = TileState::Dirty;
        }
    }

    pub fn invalidate_tile(&mut self, tile_id: TileId) {
        if let Some(tile) = self.arena.get_mut(&tile_id) {
            tile.state = TileState::Dirty;
        }
    }

    pub fn get_tile_mut(&mut self, tile_id: TileId) -> Option<&mut Tile> {
        self.arena.get_mut(&tile_id)
    }

    pub fn get_tile(&self, tile_id: TileId) -> Option<&Tile> {
        self.arena.get(&tile_id)
    }

    /// The rows each layer has tiles for, as the top of the first and the bottom of the last.
    /// A band is tiled whole, so every row between has tiles too.
    pub fn tiled_rows(&self) -> HashMap<LayerId, (f64, f64)> {
        let tile_h = self.default_tile_dimension.height;
        self.tiles
            .iter()
            .filter_map(|(&layer_id, layer)| {
                let (&(first, _), _) = layer.cells.first_key_value()?;
                let (&(last, _), _) = layer.cells.last_key_value()?;
                Some((layer_id, (first as f64 * tile_h, (last + 1) as f64 * tile_h)))
            })
            .collect()
    }

    /// The tiles of `layer_id` that exist and touch `viewport`, edges included, in row order.
    pub fn get_intersecting_tiles(&self, layer_id: LayerId, viewport: Rect) -> Vec<TileId> {
        let Some(tile_layer) = self.tiles.get(&layer_id) else {
            return vec![];
        };
        let tile_w = self.default_tile_dimension.width;
        let tile_h = self.default_tile_dimension.height;
        if tile_w <= 0.0 || tile_h <= 0.0 {
            return vec![];
        }

        // Row r spans r*h..=(r+1)*h, so it touches y..=y2 from row ceil(y/h)-1 up to floor(y2/h).
        let row_lo = ((viewport.y / tile_h).ceil() - 1.0).max(0.0) as usize;
        let row_hi = ((viewport.y + viewport.height) / tile_h).floor();
        if row_hi < 0.0 {
            return vec![];
        }
        let row_hi = row_hi as usize;
        let col_lo = ((viewport.x / tile_w).ceil() - 1.0).max(0.0) as usize;
        let col_hi = (viewport.x + viewport.width) / tile_w;
        if col_hi < 0.0 || row_lo > row_hi {
            return vec![];
        }
        let col_hi = col_hi.floor() as usize;

        tile_layer
            .cells
            .range((row_lo, 0)..=(row_hi, usize::MAX))
            .filter(|((_, col), _)| (col_lo..=col_hi).contains(col))
            .map(|(_, id)| *id)
            .collect()
    }
}

impl TileList {
    pub fn new(layer_list: LayerList, dimension: Dimension) -> Self {
        Self {
            layer_list: Arc::new(layer_list),
            tiles: HashMap::new(),
            arena: HashMap::new(),
            index: None,
            painted: Vec::new(),
            next_node_id: Arc::new(RwLock::new(TileId::new(0))),
            default_tile_dimension: dimension,
            shape_cache: Arc::new(ShapeCache::new()),
        }
    }

    /// Like `new`, but reuses an `Arc`ed `LayerList`: the hover-repaint fast path
    /// needs this to avoid cloning the layout tree.
    pub fn from_arc(layer_list: Arc<LayerList>, dimension: Dimension) -> Self {
        Self {
            layer_list,
            tiles: HashMap::new(),
            arena: HashMap::new(),
            index: None,
            painted: Vec::new(),
            next_node_id: Arc::new(RwLock::new(TileId::new(0))),
            default_tile_dimension: dimension,
            shape_cache: Arc::new(ShapeCache::new()),
        }
    }

    /// Tile the whole page. The tile count grows with the page, so only tools and tests that
    /// render a page whole call this; a browsing pass calls [`Self::cover`] with its band.
    pub fn generate(&mut self) {
        self.tiles.clear();
        self.arena.clear();
        self.painted.clear();
        self.cover(TileBand::Page);
    }

    /// Make the grid hold exactly the tiles of `band`: tiles outside it are dropped, missing
    /// ones are created `Dirty` with their elements assigned, and the rest stay as they are.
    ///
    /// Tiles exist only where a pass needs them, so what is allocated depends on the viewport,
    /// not on how long the page is. Pixels already baked for rows outside the band are not the
    /// grid's business: they live in the baked tiles, which the tile budget evicts.
    ///
    /// Per layer, the band is:
    /// - scrolling layers: the band's rows.
    /// - `position: fixed`: the viewport at scroll 0, since a fixed tile's page position is its
    ///   viewport position.
    /// - `position: sticky`: the rows that land in the viewport for any scroll position the band
    ///   covers. The catch-up offset never grows faster than the scroll does, so the ends of the
    ///   band bound it.
    pub fn cover(&mut self, band: TileBand) {
        if self.default_tile_dimension.width == 0.0 || self.default_tile_dimension.height == 0.0 {
            log::error!("Tile dimension is zero, cannot generate tiles");
            return;
        }
        let tile_w = self.default_tile_dimension.width;
        let tile_h = self.default_tile_dimension.height;

        let index = match &self.index {
            Some(index) => Arc::clone(index),
            None => {
                let index = Arc::new(self.build_index());
                self.index = Some(Arc::clone(&index));
                index
            }
        };

        for layer in &index.layers {
            let layer_id = layer.layer_id;
            let (rows, cols) = match layer.extent {
                Some((rs, re, cs, ce)) => {
                    let (lo, hi) = band_rows(layer.anchor, band);
                    let first = ((lo / tile_h).floor().max(0.0) as usize).max(rs);
                    let last = ((hi / tile_h).ceil().max(0.0) as usize).min(re);
                    (first..last, cs..ce)
                }
                None => (0..0, 0..0),
            };

            // Most layers of a long page are nowhere near the band; those cost a lookup.
            if rows.is_empty() || cols.is_empty() {
                if let Some(gone) = self.tiles.remove(&layer_id) {
                    for tile_id in gone.cells.values() {
                        self.arena.remove(tile_id);
                    }
                }
                continue;
            }

            let tile_layer = self.tiles.entry(layer_id).or_insert_with(|| TileLayer {
                layer_id,
                cells: BTreeMap::new(),
            });
            let arena = &mut self.arena;
            tile_layer.cells.retain(|(row, _), tile_id| {
                let keep = rows.contains(row);
                if !keep {
                    arena.remove(tile_id);
                }
                keep
            });

            for row in rows {
                for col in cols.clone() {
                    let std::collections::btree_map::Entry::Vacant(cell) = tile_layer.cells.entry((row, col)) else {
                        continue;
                    };
                    let tile_id = take_id(&self.next_node_id);
                    let rect = Rect::new(col as f64 * tile_w, row as f64 * tile_h, tile_w, tile_h);
                    let tile = Tile {
                        id: tile_id,
                        layer_id,
                        state: TileState::Dirty,
                        elements: tile_elements(layer, rect),
                        texture_id: None,
                        rect,
                        bgcolor: index.bgcolor,
                    };
                    arena.insert(tile_id, tile);
                    cell.insert(tile_id);
                }
            }
        }
    }

    /// Index every layer's elements: which cells the layer can have tiles in, and which elements
    /// paint into a given tile.
    fn build_index(&self) -> GridIndex {
        let tile_w = self.default_tile_dimension.width;
        let tile_h = self.default_tile_dimension.height;
        let page_w = self.layer_list.layout_tree.root_dimension.width;
        let page_h = self.layer_list.layout_tree.root_dimension.height;
        let max_cols = (page_w / tile_w).ceil().max(0.0) as usize;
        let max_rows = (page_h / tile_h).ceil().max(0.0) as usize;

        // The whole canvas is painted with the background color of the html node, falling back to body.
        let doc = self.layer_list.layout_tree.render_tree.doc.as_ref();
        let bgcolor = get_background_color_from_node(doc.html_node_id(), doc)
            .or_else(|| get_background_color_from_node(doc.body_node_id(), doc));

        let layer_list = self.layer_list.layers.read();
        let mut layers = Vec::new();
        for (layer_idx, layer_id) in self.layer_list.layer_ids.read().iter().enumerate() {
            let Some(layer) = layer_list.get(layer_id) else {
                continue;
            };

            // Only tile the union bounding box of the layer's elements. Layer 0 is the exception:
            // it carries the canvas background color, so it needs full-page coverage.
            let extent = if layer_idx == 0 || layer.elements.is_empty() {
                Some((0, max_rows, 0, max_cols))
            } else {
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                for &eid in &layer.elements {
                    if let Some(el) = self.layer_list.layout_tree.get_node_by_id(eid) {
                        let m = paint_extent(el);
                        if m.width > 0.0 && m.height > 0.0 {
                            min_x = min_x.min(m.x);
                            min_y = min_y.min(m.y);
                            max_x = max_x.max(m.x + m.width);
                            max_y = max_y.max(m.y + m.height);
                        }
                    }
                }
                if min_x > max_x || min_y > max_y {
                    None
                } else {
                    // Clamp BOTH ends into the page's tile grid: elements can sit entirely
                    // outside the page box (negative text-indent / off-screen positioning),
                    // which would otherwise put start past the clamped end.
                    let cs = ((min_x / tile_w).floor().max(0.0) as usize).min(max_cols);
                    let ce = ((max_x / tile_w).ceil().max(0.0) as usize).min(max_cols);
                    let rs = ((min_y / tile_h).floor().max(0.0) as usize).min(max_rows);
                    let re = ((max_y / tile_h).ceil().max(0.0) as usize).min(max_rows);
                    Some((rs, re, cs, ce))
                }
            };
            // Empty after clamping = the layer lies entirely outside the page box.
            let extent = extent.filter(|&(rs, re, cs, ce)| rs < re && cs < ce);

            let mut entries = Vec::with_capacity(layer.elements.len());
            for (order, &element_id) in layer.elements.iter().enumerate() {
                let Some(element) = self.layer_list.layout_tree.get_node_by_id(element_id) else {
                    log::warn!("Warning: Element {:?} not found in layout tree!", element_id);
                    continue;
                };
                // An outline and an outer shadow paint outside the margin box; without this they
                // are clipped at the edge of a tile the element isn't assigned to.
                let mut margin_box = paint_extent(element);
                let ext = ink_extent(&*self.layer_list.layout_tree.render_tree.doc, element.dom_node_id);
                if ext > 0.0 {
                    margin_box = Rect::new(
                        margin_box.x - ext,
                        margin_box.y - ext,
                        margin_box.width + ext * 2.0,
                        margin_box.height + ext * 2.0,
                    );
                }
                entries.push(GeomWithData::new(
                    rstar::primitives::Rectangle::from_corners(
                        [margin_box.x, margin_box.y],
                        [margin_box.x + margin_box.width, margin_box.y + margin_box.height],
                    ),
                    (order, element_id, margin_box),
                ));
            }

            layers.push(LayerIndex {
                layer_id: *layer_id,
                anchor: layer.anchor,
                extent,
                elements: rstar::RTree::bulk_load(entries),
            });
        }
        GridIndex { bgcolor, layers }
    }

    pub fn print_list(&self) {
        println!("Generated tilelist:");
        for (layer_id, tile_layer) in self.tiles.iter() {
            println!("Layer: {}", layer_id);
            for tile_id in tile_layer.cells.values() {
                let Some(tile) = self.arena.get(tile_id) else {
                    continue;
                };
                println!("  Tile: {} : {} elements", tile_id, tile.elements.len());
            }
        }
    }

    pub fn next_node_id(&self) -> TileId {
        take_id(&self.next_node_id)
    }
}

/// Page-space rows `band` asks of a layer anchored by `anchor`, before clamping to its extent.
fn band_rows(anchor: TileAnchor, band: TileBand) -> (f64, f64) {
    let TileBand::Rows { lo, hi, vp_h } = band else {
        return (0.0, f64::INFINITY);
    };
    match anchor {
        TileAnchor::Scroll => (lo, hi),
        TileAnchor::Fixed => (0.0, vp_h),
        TileAnchor::Sticky(constraint) => {
            // A band `lo..hi` is the viewport at every scroll in `lo..=hi - vp_h`. At scroll `s`
            // the viewport shows page rows `s - dy(s)..s + vp_h - dy(s)` of the layer, and both
            // ends only move down as `s` grows.
            let first = lo;
            let last = (hi - vp_h).max(lo);
            let dy = |scroll: f64| constraint.offset(0.0, scroll).1;
            (first - dy(first), last + vp_h - dy(last))
        }
    }
}

fn take_id(next: &RwLock<TileId>) -> TileId {
    let mut nid = next.write();
    let id = *nid;
    *nid += 1;
    id
}

/// The elements of a layer that paint into the tile at `tile_rect`, in paint order, each cut to
/// the part inside the tile. Edges count as touching, as they always have.
fn tile_elements(index: &LayerIndex, tile_rect: Rect) -> Vec<TiledLayoutElement> {
    let mut hits: Vec<_> = index
        .elements
        .locate_in_envelope_intersecting(AABB::from_corners(
            [tile_rect.x, tile_rect.y],
            [tile_rect.x + tile_rect.width, tile_rect.y + tile_rect.height],
        ))
        .map(|entry| entry.data)
        .collect();
    hits.sort_unstable_by_key(|&(order, _, _)| order);

    hits.into_iter()
        .map(|(_, element_id, margin_box)| {
            let position = Coordinate::new(
                margin_box.x.max(tile_rect.x) - tile_rect.x,
                margin_box.y.max(tile_rect.y) - tile_rect.y,
            );
            let rect = Rect::new(
                tile_rect.x.max(margin_box.x) - margin_box.x,
                tile_rect.y.max(margin_box.y) - margin_box.y,
                (tile_rect.x + tile_rect.width).min(margin_box.x + margin_box.width) - tile_rect.x.max(margin_box.x),
                (tile_rect.y + tile_rect.height).min(margin_box.y + margin_box.height) - tile_rect.y.max(margin_box.y),
            );
            TiledLayoutElement {
                id: element_id,
                rect,
                position,
                paint_commands: vec![],
            }
        })
        .collect()
}

/// How far an element's paint reaches beyond its border box: a visible outline (width +
/// positive offset) or an outer shadow (offset + spread + blur), whichever goes further. Must
/// agree with `Painter::outline_command` and with
/// [`Shadow::ink_rect`](crate::painter::commands::shadow::Shadow::ink_rect).
fn ink_extent(doc: &dyn PipelineDocument, node_id: NodeId) -> f64 {
    let style = doc.computed_style(node_id);
    let outline = &style.outline;
    let width = f64::from(outline.width);
    let outline_reach = if width <= 0.0 || !outline.style.is_visible() {
        0.0
    } else {
        width + f64::from(outline.offset).max(0.0)
    };
    style
        .background
        .shadows
        .iter()
        .filter(|shadow| !shadow.inset)
        .map(|shadow| {
            let offset = shadow.offset_x.abs().max(shadow.offset_y.abs());
            f64::from(offset + shadow.spread.max(0.0)) + blur_reach(f64::from(shadow.blur) / 2.0)
        })
        .fold(outline_reach, f64::max)
}

fn get_background_color_from_node(node_id: Option<NodeId>, doc: &dyn PipelineDocument) -> Option<(f32, f32, f32, f32)> {
    let node_id = node_id?;
    let color = doc.computed_style(node_id).background.color;
    let alpha = f32::from(color.a) / 255.0;
    (alpha > 0.0).then(|| {
        (
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
            alpha,
        )
    })
}
