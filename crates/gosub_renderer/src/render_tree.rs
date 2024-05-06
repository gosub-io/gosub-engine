use taffy::{Layout, TaffyTree};
use taffy::{NodeId as TaffyID, NodeId};
use url::Url;
use winit::dpi::PhysicalSize;

use gosub_html5::node::NodeId as GosubID;
use gosub_rendering::position::PositionTree;
use gosub_styling::css_values::CssProperties;
use gosub_styling::render_tree::{RenderNodeData, RenderTree as StyleTree};

pub type NodeID = TaffyID;

pub struct TreeDrawer {
    pub(crate) style: StyleTree,
    pub(crate) root: NodeID,
    pub(crate) taffy: TaffyTree<GosubID>,
    pub(crate) size: Option<PhysicalSize<u32>>,
    pub(crate) url: Url,
    pub(crate) position: PositionTree,
    pub(crate) last_hover: Option<NodeId>,
}

impl TreeDrawer {
    pub fn new(style: StyleTree, taffy: TaffyTree<GosubID>, root: TaffyID, url: Url) -> Self {
        let position = PositionTree::from_taffy(&taffy, root);
        Self {
            style,
            root,
            taffy,
            size: None,
            url,
            position,
            last_hover: None,
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
