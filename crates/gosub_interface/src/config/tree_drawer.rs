use crate::config::HasDrawComponents;
use crate::draw::TreeDrawer;

pub trait HasTreeDrawer: HasDrawComponents {
    type TreeDrawer: TreeDrawer<Self>;
}
