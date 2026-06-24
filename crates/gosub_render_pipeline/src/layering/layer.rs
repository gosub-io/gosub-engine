use crate::common::document::node::NodeId;
use crate::common::document::style::{lookup, StyleProperty, Value};
use crate::layouter::{LayoutElementId, LayoutTree};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::ops::AddAssign;
use std::sync::Arc;

/// ID for layers
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
    /// Layer ID
    pub layer_id: LayerId,
    /// Order of the layer
    #[allow(unused)]
    pub order: isize,
    /// Group opacity applied to the whole layer at composite time (1.0 = fully opaque). Set when
    /// the layer is promoted because its source element has CSS `opacity < 1` (a compositing
    /// group): the layer's tiles are rasterized normally and the compositor fades them as a unit.
    pub opacity: f32,
    /// Elements in this layer
    pub elements: Vec<LayoutElementId>,
}

impl Layer {
    pub fn new(layer_id: LayerId, order: isize) -> Layer {
        Layer {
            layer_id,
            order,
            opacity: 1.0,
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
    /// Wrapped layout tree
    pub layout_tree: Arc<LayoutTree>,
    /// List of all (unique) layer IDs
    pub layer_ids: RwLock<Vec<LayerId>>,
    /// List of actual layers
    pub layers: RwLock<HashMap<LayerId, Layer>>,
    /// Next layer ID
    next_layer_id: RwLock<LayerId>,
    /// DOM nodes whose paint should NOT receive per-element opacity, because they live in an
    /// opacity compositing group that fades the whole layer once at composite time. The painter
    /// checks this to avoid darkening such elements twice. See [`LayerList::is_opacity_grouped`].
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
    /// Find the element at the given coordinates. It will return the given element if it is found or None otherwise
    pub fn find_element_at(&self, x: f64, y: f64) -> Option<LayoutElementId> {
        // This assumes that the layers are ordered from top to bottom
        for layer_id in self.layer_ids.read().iter().rev() {
            let binding = self.layers.read();
            let Some(layer) = binding.get(layer_id) else {
                continue;
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

    /// Creates a new fully-opaque layer at the given order and returns its id.
    pub fn new_layer(&self, order: isize) -> LayerId {
        self.new_layer_with_opacity(order, 1.0)
    }

    /// Creates a new layer at the given order with a group opacity and returns its id.
    pub fn new_layer_with_opacity(&self, order: isize, opacity: f32) -> LayerId {
        let mut layer = Layer::new(self.next_layer_id(), order);
        layer.opacity = opacity;
        let layer_id = layer.layer_id;
        self.layer_ids.write().push(layer_id);
        self.layers.write().insert(layer_id, layer);

        layer_id
    }

    /// Group opacity for a layer (1.0 if the layer is unknown or fully opaque). Used by the
    /// compositor to fade an opacity-promoted layer's tiles as a unit.
    pub fn layer_opacity(&self, layer_id: LayerId) -> f32 {
        self.layers.read().get(&layer_id).map(|l| l.opacity).unwrap_or(1.0)
    }

    /// True when this DOM node's paint must skip per-element opacity because it belongs to an
    /// opacity compositing group (the whole layer is faded once at composite time instead).
    pub fn is_opacity_grouped(&self, node_id: NodeId) -> bool {
        self.opacity_group_nodes.read().contains(&node_id)
    }

    /// Append an element to a layer, logging if the layer id is somehow missing.
    fn add_to_layer(&self, layer_id: LayerId, element_id: LayoutElementId) {
        if let Some(mut layers) = self.get_layer_mut(layer_id) {
            if let Some(layer) = layers.get_mut(&layer_id) {
                layer.add_element(element_id);
            } else {
                log::warn!("Layer {} not found in HashMap", layer_id);
            }
        }
    }

    #[allow(unused)]
    fn get_layer(&self, layer_id: LayerId) -> Option<parking_lot::RwLockReadGuard<'_, HashMap<LayerId, Layer>>> {
        let layers = self.layers.read();
        if layers.contains_key(&layer_id) {
            Some(layers)
        } else {
            None
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

        self.traverse(default_layer_id, root_id, false, false);
    }

    /// Walk the layout tree assigning each element to a layer.
    ///
    /// An element is *promoted* to its own layer (taking its whole subtree with it) when it
    /// establishes a stacking context we handle: CSS `opacity < 1` (the layer is faded as a group)
    /// or `position: fixed` (an independent/overlay layer). `in_promoted_group` is true while we
    /// are inside such a subtree; there we deliberately do NOT spin off separate image layers, so
    /// the image stays in the group layer and moves/fades together with it instead of floating on
    /// top at full opacity. `group_faded` is true only when the enclosing group's layer has
    /// `opacity < 1`; it gates the per-element opacity skip (see [`LayerList::is_opacity_grouped`]).
    fn traverse(
        &self,
        layer_id: LayerId,
        layout_element_node_id: LayoutElementId,
        in_promoted_group: bool,
        group_faded: bool,
    ) {
        let Some(layout_element) = self.layout_tree.get_node_by_id(layout_element_node_id) else {
            return;
        };
        let doc = &self.layout_tree.render_tree.doc;

        // Read the element's OWN (non-inherited) opacity and position: only the element that
        // declares the stacking context establishes the group; descendants inherit the result
        // through the layer and must not each re-promote.
        let own_opacity = match doc.get_own_style(layout_element.dom_node_id, &StyleProperty::Opacity) {
            Some(Value::Number(n)) | Some(Value::Unit(n, _)) => n,
            _ => 1.0,
        };
        let is_fixed = matches!(
            doc.get_own_style(layout_element.dom_node_id, &StyleProperty::Position),
            Some(Value::Keyword(id)) if lookup(id) == "fixed"
        );

        // Promote a not-yet-grouped element with opacity < 1 or position: fixed to its own layer
        // and pull its whole subtree into that layer. The layer's opacity (1.0 for an opaque fixed
        // element) is realised at composite time. (Nested groups are not separately promoted in
        // this pass; a descendant's own opacity still applies per-element on top of any group fade.)
        // TODO: derive layer order from the element's CSS z-index / stacking context instead of 1.
        if !in_promoted_group && (own_opacity < 1.0 || is_fixed) {
            let layer_opacity = own_opacity.clamp(0.0, 1.0);
            let group_layer_id = self.new_layer_with_opacity(1, layer_opacity);
            self.add_to_layer(group_layer_id, layout_element.id);
            let faded = layer_opacity < 1.0;
            // Only a faded layer risks double-darkening, so only then skip the element's per-element
            // opacity (here the promoting element's own opacity is what the layer fade realises).
            if faded {
                self.opacity_group_nodes.write().insert(layout_element.dom_node_id);
            }
            for &child_id in &layout_element.children {
                self.traverse(group_layer_id, child_id, true, faded);
            }
            return;
        }

        let is_image = doc
            .tag_name(layout_element.dom_node_id)
            .map(|tag| tag.eq_ignore_ascii_case("img"))
            .unwrap_or(false);

        if is_image && !in_promoted_group {
            // Standalone image: give it its own layer (existing behaviour).
            let image_layer_id = self.new_layer(1);
            self.add_to_layer(image_layer_id, layout_element.id);
        } else {
            self.add_to_layer(layer_id, layout_element.id);
            // Inside a faded group, an element with no own opacity relies entirely on the layer
            // fade, so its paint skips per-element opacity. An element that *does* declare its own
            // opacity keeps applying it per-element (an approximation that stacks with the fade).
            if in_promoted_group && group_faded && own_opacity >= 1.0 {
                self.opacity_group_nodes.write().insert(layout_element.dom_node_id);
            }
        }

        for &child_id in &layout_element.children {
            self.traverse(layer_id, child_id, in_promoted_group, group_faded);
        }
    }

    fn next_layer_id(&self) -> LayerId {
        let mut nid = self.next_layer_id.write();
        let id = *nid;
        *nid += 1;
        id
    }
}
