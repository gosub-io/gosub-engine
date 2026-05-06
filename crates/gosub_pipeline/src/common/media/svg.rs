use crate::common::geo::Dimension;
use resvg::usvg;
use std::sync::{Arc, RwLock};

/// Cached render of an SVG at a specific dimension.
pub struct RenderedSvg {
    pub dimension: Dimension,
    pub data: Vec<u8>,
}

#[derive(Clone)]
pub struct Svg {
    pub tree: usvg::Tree,
    /// Rendered cache — dimension and pixel data kept under one lock for consistency.
    pub rendered: Arc<RwLock<RenderedSvg>>,
}

impl Svg {
    #[allow(unused)]
    pub fn new(tree: usvg::Tree) -> Svg {
        Svg {
            tree,
            rendered: Arc::new(RwLock::new(RenderedSvg {
                dimension: Dimension::ZERO,
                data: vec![],
            })),
        }
    }
}

impl std::fmt::Debug for Svg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Svg").field("tree", &self.tree).finish()
    }
}
