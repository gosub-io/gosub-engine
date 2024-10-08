use std::cmp::Ordering;

use rstar::{RTree, RTreeObject, AABB};

use crate::render_tree::RenderTree;
use gosub_render_backend::layout::{Layout, LayoutTree, Layouter};
use gosub_render_backend::RenderBackend;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::CssSystem;
use gosub_shared::traits::document::Document;

#[derive(Debug)]
pub struct Element {
    id: NodeId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: Option<(f32, f32, f32, f32)>,
    z_index: i32,
}

impl RTreeObject for Element {
    type Envelope = AABB<(f32, f32)>;
    fn envelope(&self) -> Self::Envelope {
        let lower = (self.x, self.y);
        let upper = (self.x + self.width, self.y + self.height);
        AABB::from_corners(lower, upper)
    }
}

#[derive(Default, Debug)]
pub struct PositionTree {
    tree: RTree<Element>,
}

impl PositionTree {
    pub fn from_tree<B: RenderBackend, L: Layouter, D: Document<C>, C: CssSystem>(
        from_tree: &RenderTree<L, C>,
    ) -> Self {
        let mut tree = RTree::new();

        //TODO: we somehow need to get the border radius and a potential stacking context of the element here

        Self::add_node_to_tree::<L, D, C>(from_tree, from_tree.root, 0, &mut tree, (0.0, 0.0));

        Self { tree }
    }

    fn add_node_to_tree<L: Layouter, D: Document<C>, C: CssSystem>(
        from_tree: &RenderTree<L, C>,
        id: NodeId,
        z_index: i32,
        tree: &mut RTree<Element>,
        mut pos: (f32, f32),
    ) {
        let Some(layout) = from_tree.get_layout(id) else {
            return;
        };

        let p = layout.rel_pos();

        pos.0 += p.x;
        pos.1 += p.y;

        let size = layout.size();
        let element = Element {
            id,
            x: pos.0,
            y: pos.1,
            width: size.width,
            height: size.height,
            radius: None, //TODO: border radius
            z_index,
        };

        tree.insert(element);

        for child in from_tree.children(id).unwrap_or_default() {
            Self::add_node_to_tree::<L, D, C>(from_tree, child, z_index + 1, tree, pos);
        }
    }

    pub fn find(&self, x: f32, y: f32) -> Option<NodeId> {
        let envelope = AABB::from_point((x, y));

        self.tree
            .locate_in_envelope_intersecting(&envelope)
            .filter(|e| {
                let Some(radi) = e.radius else {
                    return true;
                };

                let middle = (e.x + e.width / 2.0, e.y + e.height / 2.0);

                match middle.0.total_cmp(&x) {
                    Ordering::Equal => true,
                    Ordering::Less => {
                        match middle.1.total_cmp(&y) {
                            Ordering::Equal => true,
                            // top left
                            Ordering::Less => {
                                if (e.x + radi.0) > x && (e.y + radi.0) > y {
                                    return is_point_in_circle((e.x + radi.0, e.y + radi.0), radi.0, (x, y));
                                }
                                false
                            }
                            // top right
                            Ordering::Greater => {
                                if (e.x + e.width - radi.1) < x && (e.y + radi.1) < y {
                                    return is_point_in_circle((e.x + radi.1, e.y + radi.1), radi.1, (x, y));
                                }

                                false
                            }
                        }
                    }
                    Ordering::Greater => {
                        match middle.1.total_cmp(&y) {
                            Ordering::Equal => true,
                            // bottom left
                            Ordering::Less => {
                                if (e.x + radi.2) > x && (e.y + e.height - radi.2) < y {
                                    return is_point_in_circle((e.x + radi.2, e.y + e.height - radi.2), radi.2, (x, y));
                                }
                                false
                            }
                            // bottom right
                            Ordering::Greater => {
                                if (e.x + e.width - radi.3) < x && (e.y + e.height - radi.3) < y {
                                    return is_point_in_circle((e.x + radi.3, e.y + radi.3), radi.3, (x, y));
                                }
                                false
                            }
                        }
                    }
                }
            })
            .reduce(|a, b| if a.z_index >= b.z_index { a } else { b }) // >= because we just hope that the last-drawn element is last in the list
            .map(|e| e.id)
    }

    pub fn get_node(&self, id: NodeId) -> Option<&Element> {
        self.tree.iter().find(|e| e.id == id)
    }

    pub fn position(&self, id: NodeId) -> Option<(f32, f32)> {
        self.get_node(id).map(|e| (e.x, e.y))
    }
}

fn is_point_in_circle(circle_center: (f32, f32), circle_radius: f32, point: (f32, f32)) -> bool {
    let dx = circle_center.0 - point.0;
    let dy = circle_center.1 - point.1;
    let distance = (dx * dx + dy * dy).sqrt();

    distance <= circle_radius
}
