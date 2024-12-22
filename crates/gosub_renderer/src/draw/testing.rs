use crate::draw::TreeDrawerImpl;
use gosub_interface::config::HasDrawComponents;
use gosub_interface::css3::{CssPropertyMap, CssValue};
use gosub_rendering::render_tree::{RenderNodeData, RenderTree, TextData};
use gosub_shared::node::NodeId;

pub(crate) fn test_add_element<C: HasDrawComponents<RenderTree = RenderTree<C>, LayoutTree = RenderTree<C>>>(
    d: &mut TreeDrawerImpl<C>,
) {
    d.dirty = true;

    let mut props = C::CssPropertyMap::default();

    props.insert("width", C::CssValue::new_unit(100.0, "px".to_string()).into());
    props.insert("height", C::CssValue::new_unit(100.0, "px".to_string()).into());
    props.insert(
        "background-color",
        C::CssValue::new_color(255.0, 0.0, 0.0, 255.0).into(),
    );

    let id = d
        .tree
        .insert_element(NodeId::from(14u64), "div".to_string(), None, props);

    d.tree.layout_dirty_from(NodeId::from(14u64));

    let mut props = C::CssPropertyMap::default();

    props.insert("font-size", C::CssValue::new_number(16.0).into());
    props.insert("color", C::CssValue::new_color(0.0, 1.0, 1.0, 1.0).into());

    d.tree.insert_node_data(
        id,
        "#text".to_string(),
        RenderNodeData::Text(Box::new(TextData {
            text: "test add element".to_string(),
            layout: None,
        })),
        props,
    );

    d.debugger_scene = None;
    d.tree_scene = None;
}

pub(crate) fn test_restyle_element<C: HasDrawComponents>(_d: &mut TreeDrawerImpl<C>) {
    todo!()
}
