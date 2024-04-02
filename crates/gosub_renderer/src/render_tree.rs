use taffy::NodeId as TaffyID;
use taffy::{Layout, TaffyTree};
use url::Url;
use winit::dpi::PhysicalSize;

use gosub_html5::node::NodeId as GosubID;
use gosub_styling::render_tree::{RenderNodeData, RenderTree as StyleTree};
use gosub_styling::styling::CssProperties;

pub type NodeID = TaffyID;

pub struct TreeDrawer {
    pub(crate) style: StyleTree,
    pub(crate) root: NodeID,
    pub(crate) taffy: TaffyTree<GosubID>,
    pub(crate) size: Option<PhysicalSize<u32>>,
    pub(crate) url: Url,
}

impl TreeDrawer {
    pub fn new(style: StyleTree, taffy: TaffyTree<GosubID>, root: TaffyID, url: Url) -> Self {
        Self {
            style,
            root,
            taffy,
            size: None,
            url,
        }
    }
}

pub struct RenderTreeNode {
    pub parent: Option<NodeID>,
    pub children: Vec<NodeID>,
    pub layout: Layout,
    pub name: String,
    pub properties: CssProperties,
    pub namespace: Option<String>,
    pub data: RenderNodeData,
}
