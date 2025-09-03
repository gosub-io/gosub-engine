use std::collections::HashMap;
use std::ops::AddAssign;
use std::sync::{Arc, RwLock};
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use crate::layouter::{LayoutElementId, LayoutTree};

/// ID for layers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(u64);

impl LayerId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

impl AddAssign<i32> for LayerId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
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
    /// Elements in this layer
    pub elements: Vec<LayoutElementId>
}

impl Layer {
    pub fn new(layer_id: LayerId, order: isize) -> Layer {
        Layer {
            layer_id,
            order,
            elements: Vec::new()
        }
    }

    fn add_element(&mut self, element_id: LayoutElementId) {
        self.elements.push(element_id);
    }
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("elements", &self.elements)
            .finish()
    }
}

/// A list of layers that is returned by the pipeline stage
pub struct LayerList<C: HasDocument> {
    /// Wrapped layout tree
    pub layout_tree: Arc<LayoutTree<C>>,
    /// List of all (unique) layer IDs
    pub layer_ids: RwLock<Vec<LayerId>>,
    /// List of actual layers
    pub layers: RwLock<HashMap<LayerId, Layer>>,
    /// Next layer ID
    next_layer_id: RwLock<LayerId>,
}

impl<C: HasDocument> std::fmt::Debug for LayerList<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerList")
            .field("layout_tree", &self.layout_tree)
            .field("layers", &self.layers)
            .finish()
    }
}

impl<C: HasDocument> LayerList<C> {
    pub fn new(layout_tree: LayoutTree<C>) -> LayerList<C> {
        let mut layer_list = LayerList {
            layout_tree: Arc::new(layout_tree),
            layers: RwLock::new(HashMap::new()),
            layer_ids: RwLock::new(Vec::new()),
            next_layer_id: RwLock::new(LayerId::new(0)),
        };

        layer_list.generate_layers();
        layer_list
    }

    /// @TODO: This must be done through rstar!
    /// Find the element at the given coordinates. It will return the given element if it is found or None otherwise
    pub fn find_element_at(&self, x: f64, y: f64) -> Option<LayoutElementId> {
        // This assumes that the layers are ordered from top to bottom
        for layer_id in self.layer_ids.read().expect("Failed to lock layer IDs").iter().rev() {
            let binding = self.layers.read().expect("Failed to lock layers");
            let Some(layer) = binding.get(layer_id) else {
                continue;
            };

            for element_id in layer.elements.iter().rev() {
                let layout_element = self.layout_tree.get_node_by_id(*element_id).expect("Failed to get layout element");
                let box_model = &layout_element.box_model;

                // @TODO: use rtree for this
                if x >= box_model.margin_box.x &&
                    x < box_model.margin_box.x + box_model.margin_box.width &&
                    y >= box_model.margin_box.y &&
                    y < box_model.margin_box.y + box_model.margin_box.height
                {
                    return Some(*element_id);
                }
            }
        }

        None
    }

    // Create a new layer to the list at the given order
    pub fn new_layer(&self, order: isize) -> LayerId {
        let layer = Layer::new(self.next_layer_id(), order);
        let layer_id = layer.layer_id;
        self.layer_ids.write().expect("Failed to lock layer IDs").push(layer_id);
        self.layers.write().expect("Failed to lock layers").insert(layer_id, layer);

        layer_id
    }

    #[allow(unused)]
    fn get_layer(&self, layer_id: LayerId) -> Option<std::sync::RwLockReadGuard<HashMap<LayerId, Layer>>> {
        let layers = self.layers.read().expect("Failed to lock layers");
        if layers.contains_key(&layer_id) {
            Some(layers)
        } else {
            None
        }
    }

    fn get_layer_mut(&self, layer_id: LayerId) -> Option<std::sync::RwLockWriteGuard<HashMap<LayerId, Layer>>> {
        let layers = self.layers.write().expect("Failed to lock layers");
        if layers.contains_key(&layer_id) {
            Some(layers)
        } else {
            None
        }
    }

    fn generate_layers(&mut self) {
        self.layers.write().expect("Failed to lock layers").clear();

        let root_id = self.layout_tree.root_id;
        let default_layer_id = self.new_layer(0);

        self.traverse(default_layer_id, root_id);
    }

    fn traverse(&self, layer_id: LayerId, layout_element_node_id: LayoutElementId) {
        let Some(layout_element) = self.layout_tree.get_node_by_id(layout_element_node_id) else {
            return;
        };

        let is_image = matches!(
            self.layout_tree.render_tree.doc.node_by_id(layout_element.dom_node_id),
            Some(dom_node) if matches!(
                dom_node.node_type,
                NodeType::ElementNode(ref element_data) if element_data.tag_name.eq_ignore_ascii_case("img")
            )
        );

        // When we detect an image, we create a new layer for it
        if is_image {
            let image_layer_id = self.new_layer(1);
            if let Some(mut layers) = self.get_layer_mut(image_layer_id) {
                if let Some(image_layer) = layers.get_mut(&image_layer_id) {
                    image_layer.add_element(layout_element.id);
                } else {
                    log::warn!("Image layer {} not found in HashMap", image_layer_id);
                }
            }
        } else {
            if let Some(mut layers) = self.get_layer_mut(layer_id) {
                if let Some(layer) = layers.get_mut(&layer_id) {
                    layer.add_element(layout_element.id);
                } else {
                    log::warn!("Layer {} not found in HashMap", layer_id);
                }
            }
        }

        for &child_id in &layout_element.children {
            self.traverse(layer_id, child_id);
        }
    }

    fn next_layer_id(&self) -> LayerId {
        let mut nid = self.next_layer_id.write().expect("Failed to lock next layer ID");
        let id = *nid;
        *nid += 1;
        id
    }
}