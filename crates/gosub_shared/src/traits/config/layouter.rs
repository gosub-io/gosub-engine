use crate::render_backend::layout::{LayoutTree, Layouter};
use crate::traits::config::HasCssSystem;
use std::fmt::Debug;

pub trait HasLayouter: HasCssSystem + Debug + 'static {
    type Layouter: Layouter;
    type LayoutTree: LayoutTree<Self>;
}
