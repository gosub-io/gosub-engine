use crate::traits::config::HasDrawComponents;
use crate::traits::draw::TreeDrawer;

pub trait HasTreeDrawer: HasDrawComponents {
    type TreeDrawer: TreeDrawer<Self>;
}
