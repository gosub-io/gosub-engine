use crate::config::HasCssSystem;
use crate::font::HasFontManager;
use crate::layout::{LayoutTree, Layouter};
use std::fmt::Debug;

pub trait HasLayouter: HasFontManager + HasCssSystem + Debug + 'static {
    type Layouter: Layouter<Self>;
    type LayoutTree: LayoutTree<Self>;
}
