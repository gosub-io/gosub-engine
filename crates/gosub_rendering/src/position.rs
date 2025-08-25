use std::cmp::Ordering;

use rstar::{RTree, RTreeObject, AABB};

use gosub_interface::config::HasLayouter;
use gosub_interface::layout::{Layout, LayoutTree};

#[derive(Debug)]
pub struct Element<C: HasLayouter> {
    id: <C::LayoutTree as LayoutTree<C>>::NodeId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: Option<(f32, f32, f32, f32)>,
    z_index: i32,
}

impl<C: HasLayouter> RTreeObject for Element<C> {
    type Envelope = AABB<(f32, f32)>;
    fn envelope(&self) -> Self::Envelope {
        let lower = (self.x, self.y);
        let upper = (self.x + self.width, self.y + self.height);
        AABB::from_corners(lower, upper)
    }
}

#[derive(Debug)]
pub struct PositionTree<C: HasLayouter> {
    tree: RTree<Element<C>>,
}

impl<C: HasLayouter> Default for PositionTree<C> {
    fn default() -> Self {
        Self { tree: RTree::default() }
    }
}

impl<C: HasLayouter> PositionTree<C> {
    pub fn from_tree(from_tree: &C::LayoutTree) -> Self {
        let mut tree = RTree::new();

        //TODO: we somehow need to get the border radius and a potential stacking context of the element here

        Self::add_node_to_tree(from_tree, from_tree.root(), 0, &mut tree, (0.0, 0.0));

        Self { tree }
    }

    fn add_node_to_tree(
        from_tree: &C::LayoutTree,
        id: <C::LayoutTree as LayoutTree<C>>::NodeId,
        z_index: i32,
        tree: &mut RTree<Element<C>>,
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
            Self::add_node_to_tree(from_tree, child, z_index + 1, tree, pos);
        }
    }

    #[must_use]
    pub fn find(&self, x: f32, y: f32) -> Option<<C::LayoutTree as LayoutTree<C>>::NodeId> {
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

    pub fn get_node(&self, id: <C::LayoutTree as LayoutTree<C>>::NodeId) -> Option<&Element<C>> {
        self.tree.iter().find(|e| e.id == id)
    }

    pub fn position(&self, id: <C::LayoutTree as LayoutTree<C>>::NodeId) -> Option<(f32, f32)> {
        self.get_node(id).map(|e| (e.x, e.y))
    }
}

fn is_point_in_circle(circle_center: (f32, f32), circle_radius: f32, point: (f32, f32)) -> bool {
    let dx = circle_center.0 - point.0;
    let dy = circle_center.1 - point.1;
    let distance = (dx * dx + dy * dy).sqrt();

    distance <= circle_radius
}
