use taffy::prelude::*;

use gosub_html5::node::NodeId as GosubID;
use gosub_render_backend::RenderBackend;
use gosub_styling::render_tree::RenderTree;

use crate::style::get_style_from_node;

pub fn generate_taffy_tree<B: RenderBackend>(
    rt: &mut RenderTree<B>,
) -> anyhow::Result<(TaffyTree<GosubID>, NodeId)> {
    let mut tree: TaffyTree<GosubID> = TaffyTree::with_capacity(rt.nodes.len());

    let root = add_children_to_tree(rt, &mut tree, rt.root)?;

    Ok((tree, root))
}

fn add_children_to_tree<B: RenderBackend>(
    rt: &mut RenderTree<B>,
    tree: &mut TaffyTree<GosubID>,
    node_id: GosubID,
) -> anyhow::Result<NodeId> {
    let Some(node_children) = rt.get_children(node_id) else {
        return Err(anyhow::anyhow!("Node not found {:?}", node_id));
    };

    let mut children = Vec::with_capacity(node_children.len());

    //clone, so we can drop the borrow of RT, we would be copying the NodeID anyway, so it's not a big deal (only a few bytes)
    for child in node_children.clone() {
        match add_children_to_tree(rt, tree, child) {
            Ok(node) => children.push(node),
            Err(e) => eprintln!("Error adding child to tree: {:?}", e),
        }
    }

    let Some(node) = rt.get_node_mut(node_id) else {
        return Err(anyhow::anyhow!("Node not found"));
    };

    let style = get_style_from_node(node);

    let node = tree
        .new_with_children(style, &children)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    tree.set_node_context(node, Some(node_id))?;

    Ok(node)
}
