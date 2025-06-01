use std::sync::{Arc, RwLock};
use resvg::usvg;
use crate::common::geo::Dimension;

#[derive(Clone)]
pub struct Svg {
    pub tree: usvg::Tree,
    /// Rendered dimension of the rendered image
    pub rendered_dimension: Arc<RwLock<Dimension>>,
    /// Rendered image in the given dimension
    pub rendered_data: Arc<RwLock<Vec<u8>>>,
}

impl Svg {
    #[allow(unused)]
    pub fn new(tree: usvg::Tree) -> Svg {
        Svg {
            tree,
            rendered_dimension: Arc::new(RwLock::new(Dimension::ZERO)),
            rendered_data: Arc::new(RwLock::new(vec![])),
        }
    }
}

impl std::fmt::Debug for Svg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Svg")
            .field("tree", &self.tree)
            .field("rendered_dimension", &self.rendered_dimension)
            .finish()
    }
}
