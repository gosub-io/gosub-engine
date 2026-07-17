use crate::common::document::node::NodeId;
use crate::common::document::pipeline_doc::PipelineDocument;
use crate::common::document::style::{StyleProperty, Value};
use crate::common::geo::{Coordinate, Dimension, Rect};
use crate::common::texture::TextureId;
use crate::layering::layer::{LayerId, LayerList};
use crate::layouter::LayoutElementId;
use crate::painter::commands::PaintCommand;
use parking_lot::RwLock;
use rstar::primitives::GeomWithData;
use rstar::AABB;
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::AddAssign;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(u64);

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

/// Each layer has a list of tiles. Each tile has a list of elements that are laid out in that tile.
#[derive(Debug, Clone)]
pub struct TileLayer {
    pub layer_id: LayerId,
    pub tiles: Vec<TileId>,
    /// R* tree for fast spatial queries of tiles inside this layer
    rstar_tree: rstar::RTree<GeomWithData<rstar::primitives::Rectangle<[f64; 2]>, TileId>>,
}

impl TileLayer {
    pub fn intersects_with(&self, rect: Rect) -> Vec<TileId> {
        self.rstar_tree
            .locate_in_envelope_intersecting(AABB::from_corners(
                [rect.x, rect.y],
                [rect.x + rect.width, rect.y + rect.height],
            ))
            .map(|x| x.data)
            .collect()
    }
}

/// Main list of tiles per layer.
#[derive(Clone)]
pub struct TileList {
    pub layer_list: Arc<LayerList>,

    pub tiles: HashMap<LayerId, TileLayer>,

    pub arena: HashMap<TileId, Tile>,
    next_node_id: Arc<RwLock<TileId>>,

    pub default_tile_dimension: Dimension,
}

impl Debug for TileList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileList")
            .field("layers", &self.tiles)
            .field("arena", &self.arena)
            .field("next_node_id", &self.next_node_id)
            .field("default_tile_dimension", &self.default_tile_dimension)
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

    pub fn get_intersecting_tiles(&self, layer_id: LayerId, viewport: Rect) -> Vec<TileId> {
        let Some(tile_layer) = self.tiles.get(&layer_id) else {
            return vec![];
        };

        tile_layer.intersects_with(viewport)
    }
}

impl TileList {
    pub fn new(layer_list: LayerList, dimension: Dimension) -> Self {
        Self {
            layer_list: Arc::new(layer_list),
            tiles: HashMap::new(),
            arena: HashMap::new(),
            next_node_id: Arc::new(RwLock::new(TileId::new(0))),
            default_tile_dimension: dimension,
        }
    }

    /// Like `new`, but reuses an `Arc`ed `LayerList`: the hover-repaint fast path
    /// needs this to avoid cloning the layout tree.
    pub fn from_arc(layer_list: Arc<LayerList>, dimension: Dimension) -> Self {
        Self {
            layer_list,
            tiles: HashMap::new(),
            arena: HashMap::new(),
            next_node_id: Arc::new(RwLock::new(TileId::new(0))),
            default_tile_dimension: dimension,
        }
    }

    pub fn generate(&mut self) {
        self.tiles.clear();
        self.arena.clear();

        if self.default_tile_dimension.width == 0.0 || self.default_tile_dimension.height == 0.0 {
            log::error!("Tile dimension is zero, cannot generate tiles");
            return;
        }
        let tile_w = self.default_tile_dimension.width;
        let tile_h = self.default_tile_dimension.height;

        let page_w = self.layer_list.layout_tree.root_dimension.width;
        let page_h = self.layer_list.layout_tree.root_dimension.height;
        let max_cols = (page_w / tile_w).ceil() as usize;
        let max_rows = (page_h / tile_h).ceil() as usize;

        // The whole canvas is painted with the background color of the html node, falling back to body.
        let mut bgcolor = get_background_color_from_node(
            self.layer_list.layout_tree.render_tree.doc.html_node_id(),
            self.layer_list.layout_tree.render_tree.doc.as_ref(),
        );
        if bgcolor.is_none() {
            bgcolor = get_background_color_from_node(
                self.layer_list.layout_tree.render_tree.doc.body_node_id(),
                self.layer_list.layout_tree.render_tree.doc.as_ref(),
            );
        }

        let layer_list = self.layer_list.layers.read();

        for (layer_idx, layer_id) in self.layer_list.layer_ids.read().iter().enumerate() {
            let Some(layer) = layer_list.get(layer_id) else {
                continue;
            };

            // Only tile the union bounding box of the layer's elements. Layer 0 is the exception:
            // it carries the canvas background color, so it needs full-page coverage.
            let (row_start, row_end, col_start, col_end) = if layer_idx == 0 || layer.elements.is_empty() {
                (0, max_rows, 0, max_cols)
            } else {
                let mut min_x = f64::MAX;
                let mut min_y = f64::MAX;
                let mut max_x = f64::MIN;
                let mut max_y = f64::MIN;
                for &eid in &layer.elements {
                    if let Some(el) = self.layer_list.layout_tree.get_node_by_id(eid) {
                        let m = el.box_model.margin_box;
                        if m.width > 0.0 && m.height > 0.0 {
                            min_x = min_x.min(m.x);
                            min_y = min_y.min(m.y);
                            max_x = max_x.max(m.x + m.width);
                            max_y = max_y.max(m.y + m.height);
                        }
                    }
                }
                if min_x > max_x || min_y > max_y {
                    continue;
                }
                let cs = (min_x / tile_w).floor() as usize;
                let ce = ((max_x / tile_w).ceil() as usize).min(max_cols);
                let rs = (min_y / tile_h).floor() as usize;
                let re = ((max_y / tile_h).ceil() as usize).min(max_rows);
                (rs, re, cs, ce)
            };

            let estimated = (row_end - row_start) * (col_end - col_start);
            let mut tile_ids = Vec::with_capacity(estimated);

            for y in row_start..row_end {
                for x in col_start..col_end {
                    let tile_id = self.next_node_id();
                    let tile = Tile {
                        id: tile_id,
                        layer_id: *layer_id,
                        state: TileState::Dirty,
                        elements: Vec::new(),
                        texture_id: None,
                        rect: Rect::new(x as f64 * tile_w, y as f64 * tile_h, tile_w, tile_h),
                        bgcolor,
                    };

                    self.arena.insert(tile_id, tile);
                    tile_ids.push(tile_id);
                }
            }

            let rtree_data: Vec<_> = tile_ids
                .iter()
                .filter_map(|tile_id| {
                    let Some(tile) = self.arena.get(tile_id) else {
                        log::warn!("Tile {:?} missing from arena while building rtree", tile_id);
                        return None;
                    };
                    Some(GeomWithData::new(
                        rstar::primitives::Rectangle::from_corners(
                            [tile.rect.x, tile.rect.y],
                            [tile.rect.x + tile.rect.width, tile.rect.y + tile.rect.height],
                        ),
                        *tile_id,
                    ))
                })
                .collect();

            let tile_layer = TileLayer {
                layer_id: *layer_id,
                tiles: tile_ids.clone(),
                rstar_tree: rstar::RTree::bulk_load(rtree_data),
            };
            self.tiles.insert(*layer_id, tile_layer);

            let Some(tile_layer) = self.tiles.get(layer_id) else {
                continue;
            };

            for &element_id in &layer.elements {
                let Some(element) = self.layer_list.layout_tree.get_node_by_id(element_id) else {
                    log::warn!("Warning: Element {:?} not found in layout tree!", element_id);
                    continue;
                };
                let margin_box = element.box_model.margin_box;

                let matching_tile_ids = tile_layer.intersects_with(margin_box);
                for tile_id in &matching_tile_ids {
                    let Some(tile) = self.arena.get_mut(tile_id) else {
                        log::warn!("Tile {:?} missing from arena while assigning elements", tile_id);
                        continue;
                    };
                    let position = Coordinate::new(
                        margin_box.x.max(tile.rect.x) - tile.rect.x,
                        margin_box.y.max(tile.rect.y) - tile.rect.y,
                    );

                    let dimension = Rect::new(
                        tile.rect.x.max(margin_box.x) - margin_box.x,
                        tile.rect.y.max(margin_box.y) - margin_box.y,
                        (tile.rect.x + tile.rect.width).min(margin_box.x + margin_box.width)
                            - tile.rect.x.max(margin_box.x),
                        (tile.rect.y + tile.rect.height).min(margin_box.y + margin_box.height)
                            - tile.rect.y.max(margin_box.y),
                    );

                    let tiled_element = TiledLayoutElement {
                        id: element_id,
                        rect: dimension,
                        position,
                        paint_commands: vec![],
                    };

                    tile.elements.push(tiled_element);
                }
            }
        }
    }

    pub fn print_list(&self) {
        println!("Generated tilelist:");
        for (layer_id, tile_layer) in self.tiles.iter() {
            println!("Layer: {}", layer_id);
            for tile_id in tile_layer.tiles.iter() {
                let Some(tile) = self.arena.get(tile_id) else {
                    continue;
                };
                println!("  Tile: {} : {} elements", tile_id, tile.elements.len());
            }
        }
    }

    pub fn next_node_id(&self) -> TileId {
        let mut nid = self.next_node_id.write();
        let id = *nid;
        *nid += 1;
        id
    }
}

fn get_background_color_from_node(node_id: Option<NodeId>, doc: &dyn PipelineDocument) -> Option<(f32, f32, f32, f32)> {
    let node_id = node_id?;
    match doc.get_style(node_id, &StyleProperty::BackgroundColor) {
        Value::Color(r, g, b, a) => {
            let af = a as f32 / 255.0;
            if af > 0.0 {
                Some((r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, af))
            } else {
                None
            }
        }
        _ => None,
    }
}
