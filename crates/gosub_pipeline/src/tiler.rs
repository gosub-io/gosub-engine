use crate::painter::commands::color::Color;
use std::collections::HashMap;
use std::fmt::Debug;
use std::ops::AddAssign;
use std::sync::{Arc, RwLock};
use rstar::AABB;
use rstar::primitives::GeomWithData;
use crate::common::document::document::Document;
use crate::common::document::node::{NodeId, NodeType};
use crate::common::document::style::{Color as StyleColor, StyleProperty, StyleValue};
use crate::common::document::style::StyleProperty::BackgroundColor;
use crate::common::geo::{Coordinate, Dimension, Rect};
use crate::layering::layer::{LayerId, LayerList};
use crate::layouter::{LayoutElementId, LayoutElementNode};
use crate::painter::commands::PaintCommand;
use crate::common::texture::TextureId;

/*

TileList
    wrapped(LayerList)
    tiles: hashmap<LayerId, TileLayer>
    arena of Tile
    next_node_id
    default_tile_dimension

TileLayer
    layer_id
    tiles: Vec<TileId>
    rstar_tree

 Tile
    id
    layer_id
    elements: Vec<TiledLayoutElement>
    texture_id
    state
    rect

 TiledLayoutElement
    id
    rect
    position
    paint_commands
 */

/// Identifier for tiles
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(u64);

impl TileId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

impl AddAssign<i32> for TileId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
    }
}

impl std::fmt::Display for TileId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "TileId({})", self.0)
    }
}

/// An element that is laid out in a tile. It contains the paint commands to render the (partial)
/// element onto the tile.
#[derive(Debug, Clone)]
pub struct TiledLayoutElement {
    /// Element to layout
    pub id: LayoutElementId,
    /// Position and dimension of the element inside the tile. If the element is larger than the
    /// tile, this will be a subset of the element.
    pub rect: Rect,
    /// Coordinate of the element in the tile. It is the coordinate inside the tile where the element starts.
    pub position: Coordinate,
    /// List of paint commands to execute in order to draw this elements onto the tile
    pub paint_commands: Vec<PaintCommand>,
}

/*

Here is a box element (id 67) centered within 4 tiles. The tiles are 100x50 each.
The rect size is 100x50.

In tile 1, the rect of element 67 is (0, 0, 50, 25). The position is (50, 25)
In tile 2, the rect of element 67 is (50, 0, 50, 25). The position is (0, 25)
In tile 3, the rect of element 67 is (0, 25, 50, 25). The position is (50, 0).
In tile 4, the rect of element 67 is (50, 25, 50, 25). The position is (0, 0).

The position defines where the element will start in the tile.
The rect defines the position and dimension of the element that needs to be rendered.

In the first tile, the element starts at 50x25. Even though the element is 100x50 in side,
the rect starts at 0,0 to 50,25. Which is the top left quarter of the element.

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
    Clean,
    /// Tile texture needs a repaint
    Dirty,
    /// Tile texture cannot be rendered by this backend
    Unrenderable,
    /// Tile is clean, but it does not contain anything (ie: no texture needed)
    Empty,
}

/// Single tile in the tile list. It contains a list of elements that are laid out in the tile and
/// has the (rendered) texture that will eventually be composited onto the screen.
#[derive(Debug, Clone)]
pub struct Tile {
    /// Tile ID
    pub id: TileId,
    /// Layer id on which this tile lives
    pub layer_id: LayerId,
    /// Elements found in the tile
    pub elements: Vec<TiledLayoutElement>,
    /// Texture that this tile is rendered to. If it does not have a texture id, it's not rendered
    /// yet. Note that when the staet is DIRTY, the texture_id is still valid, but the texture needs
    /// to be repainted.
    pub texture_id: Option<TextureId>,
    /// State of the tile
    pub state: TileState,
    // Position and dimension of the tile in the layer
    pub rect: Rect,
    // Background color of the tile (actually, the background color of the whole canvas). We should use a different way to deal with this I think.
    pub bgcolor: Option<(f32, f32, f32, f32)>,
}

/// Each layer has a list of tiles. Each tile has a list of elements that are laid out in that tile.
#[derive(Debug, Clone)]
pub struct TileLayer {
    // Layer ID of this layer
    pub layer_id: LayerId,
    // List of tiles inside this layer
    pub tiles: Vec<TileId>,
    /// R* tree for fast spatial queries of tiles inside this layer
    rstar_tree: rstar::RTree<GeomWithData<rstar::primitives::Rectangle<[f64; 2]>, TileId>>,
}

impl TileLayer {
    // Find all tile ids in this layer that intersects with the given rect
    pub fn intersects_with(&self, rect: Rect) -> Vec<TileId> {
        self.rstar_tree
            .locate_in_envelope_intersecting(&AABB::from_corners(
                [rect.x, rect.y],
                [rect.x + rect.width, rect.y + rect.height]
            ))
            .map(|x| x.data)
            .collect()
    }
}

/// Main list of tiles per layer.
#[derive(Clone)]
pub struct TileList {
    /// Wrapped layer list
    pub layer_list: Arc<LayerList>,

    // Tile info per layer
    pub tiles: HashMap<LayerId, TileLayer>,

    /// Arena of layout nodes
    pub arena : HashMap<TileId, Tile>,
    /// Next node ID
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
        let tile = self.arena.get_mut(&tile_id).unwrap();
        tile.state = TileState::Dirty;
    }

    pub fn get_tile_mut(&mut self, tile_id: TileId) -> Option<&mut Tile> {
        self.arena.get_mut(&tile_id)
    }

    /// Returns a reference to the given tile or None when not found
    pub fn get_tile(&self, tile_id: TileId) -> Option<&Tile> {
        self.arena.get(&tile_id)
    }

    /// Return all the tiles for the specific layer that intersects with the given viewport
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

    // @TODO: Optimize: remove all tiles that are empty
    pub fn generate(&mut self) {
        let rows = (self.layer_list.layout_tree.root_dimension.height / self.default_tile_dimension.height).ceil() as usize;
        let cols = (self.layer_list.layout_tree.root_dimension.width / self.default_tile_dimension.width).ceil() as usize;

        // Detect canvas color. We paint the whole canvas with the background color from either the html or body nodes.
        let mut bgcolor = None;
        bgcolor = get_background_color_from_node(self.layer_list.layout_tree.render_tree.doc.html_node_id, &self.layer_list.layout_tree.render_tree.doc);
        if bgcolor.is_none() {
            bgcolor = get_background_color_from_node(self.layer_list.layout_tree.render_tree.doc.body_node_id, &self.layer_list.layout_tree.render_tree.doc);
        }

        let mut layer_list = self.layer_list.layers.read().unwrap();

        // iterate each layer
        for layer_id in self.layer_list.layer_ids.read().unwrap().iter() {
            // Each layer gets a list of tiles (rows * cols). They are stored in the arena.
            let mut tile_ids = Vec::with_capacity(rows * cols);

            // Generate tiles for this layer
            for y in 0..rows {
                for x in 0..cols {

                    let tile_id = self.next_node_id();
                    let tile = Tile {
                        id: tile_id,
                        layer_id: *layer_id,
                        state: TileState::Dirty,
                        elements: Vec::new(),
                        texture_id: None,
                        rect: Rect::new(
                            x as f64 * self.default_tile_dimension.width,
                            y as f64 * self.default_tile_dimension.height,
                            self.default_tile_dimension.width,
                            self.default_tile_dimension.height,
                        ),
                        bgcolor,
                    };

                    self.arena.insert(tile_id, tile);
                    tile_ids.push(tile_id);
                }
            }

            let rtree_data: Vec<_> = tile_ids.iter().map(|tile_id| {
                let tile = self.arena.get(tile_id).unwrap();
                GeomWithData::new(
                    rstar::primitives::Rectangle::from_corners(
                        [tile.rect.x, tile.rect.y],
                        [tile.rect.x + tile.rect.width, tile.rect.y + tile.rect.height]
                    ),
                    *tile_id
                )
            }).collect();

            // Add all remaining tiles to the tile layer
            let tile_layer = TileLayer {
                layer_id: *layer_id,
                tiles: tile_ids.clone(),
                rstar_tree: rstar::RTree::bulk_load(rtree_data),
            };
            self.tiles.insert(*layer_id, tile_layer);

            // Get elements in this layer
            let Some(layer) = layer_list.get(&layer_id) else {
                continue;
            };

            let Some(tile_layer) = self.tiles.get(&layer_id) else {
                continue;
            };

            // iterate each element in the layer
            for &element_id in &layer.elements {
                // Get element
                let Some(element) = self.layer_list.layout_tree.get_node_by_id(element_id) else {
                    log::warn!("Warning: Element {:?} not found in layout tree!", element_id);
                    continue;
                };
                let margin_box = element.box_model.margin_box;

                // Find all tile_ids that contain this element
                let matching_tile_ids = tile_layer.intersects_with(margin_box);
                for tile_id in &matching_tile_ids {
                    let tile = self.arena.get_mut(&tile_id).unwrap();
                    let position = Coordinate::new(
                        tile.rect.x.max(margin_box.x) - margin_box.x,
                        tile.rect.y.max(margin_box.y) - margin_box.y
                    );

                    let dimension = Rect::new(
                        margin_box.x.max(tile.rect.x) - tile.rect.x,
                        margin_box.y.max(tile.rect.y) - tile.rect.y,
                        (tile.rect.x + tile.rect.width).min(margin_box.x + margin_box.width) - tile.rect.x.max(margin_box.x),
                        (tile.rect.y + tile.rect.height).min(margin_box.y + margin_box.height) - tile.rect.y.max(margin_box.y),
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
                let tile = self.arena.get(tile_id).unwrap();
                println!("  Tile: {} : {} elements", tile_id, tile.elements.len());
            }
        }
    }

    pub fn next_node_id(&self) -> TileId {
        let mut nid = self.next_node_id.write().expect("Failed to lock next node ID");
        let id = *nid;
        *nid += 1;
        id
    }
}

fn get_background_color_from_node(node_id: Option<NodeId>, doc: &Document) -> Option<(f32, f32, f32, f32)> {
    let node_id = match node_id {
        Some(node_id) => node_id,
        None => {
            return None;
        }
    };

    let Some(node) = doc.get_node_by_id(node_id) else {
        return None;
    };

    let NodeType::Element(data) = &node.node_type else {
        return None;
    };

    data.styles.get_property(BackgroundColor).map(|value| {
        if let StyleValue::Color(color) = value {
            return convert_color(color);
        }
        None
    });

    None
}

fn convert_color(color: &StyleColor) -> Option<(f32, f32, f32, f32)> {
    let c = match color {
        StyleColor::Rgb(r, g, b) => Some((*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, 1.0)),
        StyleColor::Rgba(r, g, b, a) => Some((*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0, *a as f32 / 255.0)),
        StyleColor::Named(name) => {
            let c = Color::from_css(name.as_str());
            Some((c.r(), c.g(), c.b(), c.a()))
        },
    };

    // Check if the color is transparent. If so, we return None
    match c {
        Some((r, g, b, a)) => {
            if a > 0.0 {
                Some((r, g, b, a))
            } else {
                None
            }
        },
        None => None,
    }
}

