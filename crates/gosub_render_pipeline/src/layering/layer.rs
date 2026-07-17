use crate::common::document::node::NodeId;
use crate::common::document::style::{lookup, StyleProperty, Unit, Value};
use crate::layouter::{LayoutElementId, LayoutElementNode, LayoutTree};
use crate::render::backend::{StickyConstraint, TileAnchor};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::ops::AddAssign;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

impl LayerId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl AddAssign<u64> for LayerId {
    fn add_assign(&mut self, rhs: u64) {
        self.0 += rhs;
    }
}

impl std::fmt::Display for LayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "LayerId({})", self.0)
    }
}

#[derive(Clone)]
pub struct Layer {
    pub layer_id: LayerId,
    /// Stacking order (from the promoting element's `z-index`). The layer list is sorted by this
    /// so higher-`z-index` layers composite in front.
    pub order: isize,
    /// Group opacity: tiles rasterize normally and the compositor fades them as a unit.
    pub opacity: f32,
    /// How the layer responds to scroll — `Fixed` layers composite without the scroll offset.
    pub anchor: TileAnchor,
    pub elements: Vec<LayoutElementId>,
}

impl Layer {
    pub fn new(layer_id: LayerId, order: isize) -> Layer {
        Layer {
            layer_id,
            order,
            opacity: 1.0,
            anchor: TileAnchor::Scroll,
            elements: Vec::new(),
        }
    }

    fn add_element(&mut self, element_id: LayoutElementId) {
        self.elements.push(element_id);
    }
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer").field("elements", &self.elements).finish()
    }
}

/// A list of layers that is returned by the pipeline stage
pub struct LayerList {
    pub layout_tree: Arc<LayoutTree>,
    /// Sorted by stacking order; the compositor and hit-test both walk this.
    pub layer_ids: RwLock<Vec<LayerId>>,
    pub layers: RwLock<HashMap<LayerId, Layer>>,
    next_layer_id: RwLock<LayerId>,
    /// DOM nodes that must NOT get per-element opacity: their layer is faded once at composite
    /// time, so applying it twice would darken them. See [`LayerList::is_opacity_grouped`].
    opacity_group_nodes: RwLock<HashSet<NodeId>>,
}

impl std::fmt::Debug for LayerList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerList")
            .field("layout_tree", &self.layout_tree)
            .field("layers", &self.layers)
            .finish()
    }
}

impl Clone for LayerList {
    fn clone(&self) -> Self {
        Self {
            layout_tree: Arc::clone(&self.layout_tree),
            layer_ids: RwLock::new(self.layer_ids.read().clone()),
            layers: RwLock::new(self.layers.read().clone()),
            next_layer_id: RwLock::new(*self.next_layer_id.read()),
            opacity_group_nodes: RwLock::new(self.opacity_group_nodes.read().clone()),
        }
    }
}

impl LayerList {
    pub fn new(layout_tree: LayoutTree) -> LayerList {
        let mut layer_list = LayerList {
            layout_tree: Arc::new(layout_tree),
            layers: RwLock::new(HashMap::new()),
            layer_ids: RwLock::new(Vec::new()),
            next_layer_id: RwLock::new(LayerId::new(0)),
            opacity_group_nodes: RwLock::new(HashSet::new()),
        };

        layer_list.generate_layers();
        layer_list
    }

    /// @TODO: This must be done through rstar!
    /// Topmost element at the given viewport coordinates. Element boxes are in page space, so a
    /// scrolling layer is hit-tested at `viewport + scroll`, a `fixed` layer at the raw viewport.
    pub fn find_element_at(&self, vp_x: f64, vp_y: f64, scroll_x: f64, scroll_y: f64) -> Option<LayoutElementId> {
        // This assumes that the layers are ordered from top to bottom
        for layer_id in self.layer_ids.read().iter().rev() {
            let binding = self.layers.read();
            let Some(layer) = binding.get(layer_id) else {
                continue;
            };

            // Convert the viewport point into this layer's (page-space) coordinate space by
            // inverting the composite mapping `vp = page - scroll + sticky_offset`.
            let (x, y) = match layer.anchor {
                TileAnchor::Fixed => (vp_x, vp_y),
                TileAnchor::Scroll => (vp_x + scroll_x, vp_y + scroll_y),
                TileAnchor::Sticky(c) => {
                    let (dx, dy) = c.offset(scroll_x, scroll_y);
                    (vp_x + scroll_x - dx, vp_y + scroll_y - dy)
                }
            };

            for element_id in layer.elements.iter().rev() {
                let Some(layout_element) = self.layout_tree.get_node_by_id(*element_id) else {
                    log::warn!("Layout element {:?} not found during hit test", element_id);
                    continue;
                };
                let box_model = &layout_element.box_model;

                // @TODO: use rtree for this
                if x >= box_model.margin_box.x
                    && x < box_model.margin_box.x + box_model.margin_box.width
                    && y >= box_model.margin_box.y
                    && y < box_model.margin_box.y + box_model.margin_box.height
                {
                    return Some(*element_id);
                }
            }
        }

        None
    }

    /// Sticky constraint for a `position: sticky` element, else `None`. The cage should be the
    /// containing block's content box; we approximate it with the parent's, as there are no
    /// sub-scroll-containers yet. A root sticky element gets a zero-slack cage and never sticks.
    fn sticky_constraint(&self, el: &LayoutElementNode) -> Option<StickyConstraint> {
        let doc = &self.layout_tree.render_tree.doc;

        let is_sticky = matches!(
            doc.get_own_style(el.dom_node_id, &StyleProperty::Position),
            Some(Value::Keyword(id)) if lookup(id) == "sticky"
        );
        if !is_sticky {
            return None;
        }

        // Physical `top`/`left` map to these logical inset properties (see inline_style.rs).
        let inset_top = read_px(doc.get_own_style(el.dom_node_id, &StyleProperty::InsetBlockStart));
        let inset_left = read_px(doc.get_own_style(el.dom_node_id, &StyleProperty::InsetInlineStart));

        let natural = el.box_model.margin_box;
        let cage = el
            .parent
            .and_then(|pid| self.layout_tree.get_node_by_id(pid))
            .map(|p| p.box_model.content_box)
            .unwrap_or(natural);

        Some(StickyConstraint {
            inset_top,
            inset_left,
            natural_x: natural.x,
            natural_y: natural.y,
            natural_w: natural.width,
            natural_h: natural.height,
            cage_x: cage.x,
            cage_y: cage.y,
            cage_w: cage.width,
            cage_h: cage.height,
        })
    }

    /// Creates a new fully-opaque, scroll-anchored layer at the given order and returns its id.
    pub fn new_layer(&self, order: isize) -> LayerId {
        self.new_promoted_layer(order, 1.0, TileAnchor::Scroll)
    }

    pub fn new_promoted_layer(&self, order: isize, opacity: f32, anchor: TileAnchor) -> LayerId {
        let mut layer = Layer::new(self.next_layer_id(), order);
        layer.opacity = opacity;
        layer.anchor = anchor;
        let layer_id = layer.layer_id;
        self.layer_ids.write().push(layer_id);
        self.layers.write().insert(layer_id, layer);

        layer_id
    }

    /// Group opacity for a layer; 1.0 if the layer is unknown.
    pub fn layer_opacity(&self, layer_id: LayerId) -> f32 {
        self.layers.read().get(&layer_id).map(|l| l.opacity).unwrap_or(1.0)
    }

    /// Scroll anchor for a layer; `Scroll` if the layer is unknown.
    pub fn layer_anchor(&self, layer_id: LayerId) -> TileAnchor {
        self.layers.read().get(&layer_id).map(|l| l.anchor).unwrap_or_default()
    }

    /// True when this DOM node's paint must skip per-element opacity because it belongs to an
    /// opacity compositing group (the whole layer is faded once at composite time instead).
    pub fn is_opacity_grouped(&self, node_id: NodeId) -> bool {
        self.opacity_group_nodes.read().contains(&node_id)
    }

    fn add_to_layer(&self, layer_id: LayerId, element_id: LayoutElementId) {
        if let Some(mut layers) = self.get_layer_mut(layer_id) {
            if let Some(layer) = layers.get_mut(&layer_id) {
                layer.add_element(element_id);
            } else {
                log::warn!("Layer {} not found in HashMap", layer_id);
            }
        }
    }

    fn get_layer_mut(&self, layer_id: LayerId) -> Option<parking_lot::RwLockWriteGuard<'_, HashMap<LayerId, Layer>>> {
        let layers = self.layers.write();
        if layers.contains_key(&layer_id) {
            Some(layers)
        } else {
            None
        }
    }

    fn generate_layers(&mut self) {
        self.layers.write().clear();

        let root_id = self.layout_tree.root_id;
        let default_layer_id = self.new_layer(0);

        self.traverse(default_layer_id, root_id, false, false, 0);

        // Composite order = stacking order. Sort layers by their `order` (z-index level); the sort is
        // stable, so layers at the same level keep DOM/creation order (the correct tie-break for
        // equal z-index). The compositor and hit-test both walk `layer_ids` in this order.
        let layers = self.layers.read();
        self.layer_ids
            .write()
            .sort_by_key(|id| layers.get(id).map(|l| l.order).unwrap_or(0));
    }

    /// Walk the layout tree assigning each element to a layer. An element is *promoted* to its own
    /// layer (with its subtree) for a compositing reason (`opacity < 1`, `position: fixed`/`sticky`)
    /// even when nested, or once for a positioned `z-index`, which only re-levels its subtree.
    ///
    /// `in_promoted_group`: inside such a subtree, where images deliberately do NOT get their own
    /// layer so they move/fade with the group. `group_faded`: the enclosing layer has `opacity < 1`,
    /// which gates the per-element opacity skip. `inherited_order`: the enclosing stacking level.
    fn traverse(
        &self,
        layer_id: LayerId,
        layout_element_node_id: LayoutElementId,
        in_promoted_group: bool,
        group_faded: bool,
        inherited_order: isize,
    ) {
        let Some(layout_element) = self.layout_tree.get_node_by_id(layout_element_node_id) else {
            return;
        };
        let doc = &self.layout_tree.render_tree.doc;

        // OWN (non-inherited) styles only: descendants inherit the group through the layer and
        // must not each re-promote.
        let own_opacity = match doc.get_own_style(layout_element.dom_node_id, &StyleProperty::Opacity) {
            Some(Value::Number(n)) | Some(Value::Unit(n, _)) => n,
            _ => 1.0,
        };
        let is_fixed = matches!(
            doc.get_own_style(layout_element.dom_node_id, &StyleProperty::Position),
            Some(Value::Keyword(id)) if lookup(id) == "fixed"
        );
        // Sticky promotes like `fixed`, but its offset is resolved from scroll at composite time.
        let sticky = self.sticky_constraint(layout_element);

        // `z-index` only takes effect on positioned elements; `auto`/non-positioned stays at 0.
        let is_positioned = matches!(
            doc.get_own_style(layout_element.dom_node_id, &StyleProperty::Position),
            Some(Value::Keyword(id)) if matches!(lookup(id).as_str(), "relative" | "absolute" | "fixed" | "sticky")
        );
        let z_index: Option<isize> = if is_positioned {
            match doc.get_own_style(layout_element.dom_node_id, &StyleProperty::ZIndex) {
                Some(Value::Number(n)) => Some(n as isize),
                _ => None,
            }
        } else {
            None
        };
        // Stacking level for this element; the layer list is sorted by it after traversal, since
        // DOM order alone would put a `z-index: 0` layer on top of a `z-index: 1` one.
        let order = z_index.unwrap_or(inherited_order);

        // A compositing reason forces a layer even when nested, so the effect is not swallowed by
        // the parent layer; a plain `z-index` promotes once and otherwise carries `order` downward.
        let compositing = own_opacity < 1.0 || is_fixed || sticky.is_some();
        if compositing || (z_index.is_some() && !in_promoted_group) {
            let layer_opacity = own_opacity.clamp(0.0, 1.0);
            // Opacity is realised via `layer_opacity` regardless of the anchor, so a
            // sticky+opacity element still composes correctly.
            let anchor = if let Some(c) = sticky {
                TileAnchor::Sticky(c)
            } else if is_fixed {
                TileAnchor::Fixed
            } else {
                TileAnchor::Scroll
            };
            let group_layer_id = self.new_promoted_layer(order, layer_opacity, anchor);
            self.add_to_layer(group_layer_id, layout_element.id);
            let faded = layer_opacity < 1.0;
            // Only a faded layer risks double-darkening, so only then skip per-element opacity.
            if faded {
                self.opacity_group_nodes.write().insert(layout_element.dom_node_id);
            }
            for &child_id in &layout_element.children {
                self.traverse(group_layer_id, child_id, true, faded, order);
            }
            return;
        }

        let is_image = doc
            .tag_name(layout_element.dom_node_id)
            .map(|tag| tag.eq_ignore_ascii_case("img"))
            .unwrap_or(false);

        if is_image && !in_promoted_group {
            let image_layer_id = self.new_layer(order);
            self.add_to_layer(image_layer_id, layout_element.id);
        } else {
            self.add_to_layer(layer_id, layout_element.id);
            // In a faded group, an element with no own opacity relies entirely on the layer fade.
            // One that declares its own keeps applying it per-element — an approximation.
            if in_promoted_group && group_faded && own_opacity >= 1.0 {
                self.opacity_group_nodes.write().insert(layout_element.dom_node_id);
            }
        }

        for &child_id in &layout_element.children {
            self.traverse(layer_id, child_id, in_promoted_group, group_faded, order);
        }
    }

    fn next_layer_id(&self) -> LayerId {
        let mut nid = self.next_layer_id.write();
        let id = *nid;
        *nid += 1;
        id
    }
}

/// Read a CSS length inset as px, treating unitless numbers as px. `None` for `auto` and non-px
/// units — percentage/em insets aren't resolved here yet.
fn read_px(value: Option<Value>) -> Option<f64> {
    match value {
        Some(Value::Unit(v, Unit::Px)) => Some(v as f64),
        Some(Value::Number(v)) => Some(v as f64),
        _ => None,
    }
}
