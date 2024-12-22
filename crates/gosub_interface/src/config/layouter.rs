use crate::config::HasCssSystem;
use crate::layout::{LayoutTree, Layouter};
use std::fmt::Debug;

pub trait HasLayouter: HasCssSystem + Debug + 'static {
    type Layouter: Layouter;
    type LayoutTree: LayoutTree<Self>;
}
