use taffy::NodeId as TaffyID;
use taffy::{Layout, TaffyTree};

use gosub_html5::node::NodeId as GosubID;
use gosub_styling::css_values::CssProperties;
use gosub_styling::render_tree::{RenderNodeData, RenderTree as StyleTree};

pub type NodeID = TaffyID;

pub struct RenderTree {
    pub(crate) style: StyleTree,
    pub(crate) root: NodeID,
    pub(crate) taffy: TaffyTree<GosubID>,
}

impl RenderTree {
    pub fn new(style: StyleTree, taffy: TaffyTree<GosubID>, root: TaffyID) -> Self {
        Self { style, root, taffy }
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
