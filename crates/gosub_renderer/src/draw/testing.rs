use crate::render_tree::TreeDrawer;
use gosub_render_backend::layout::Layouter;
use gosub_render_backend::RenderBackend;
use gosub_rendering::render_tree::{RenderNodeData, TextData};
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::{CssProperty, CssPropertyMap, CssSystem, CssValue};
use gosub_shared::traits::document::Document;

pub(crate) fn test_add_element<B: RenderBackend, L: Layouter, D: Document<C>, C: CssSystem>(
    d: &mut TreeDrawer<B, L, D, C>,
) {
    d.dirty = true;

    let mut props = C::PropertyMap::default();

    props.insert(
        "width",
        <C::Property as CssProperty>::Value::new_unit(100.0, "px".to_string()).into(),
    );
    props.insert(
        "height",
        <C::Property as CssProperty>::Value::new_unit(100.0, "px".to_string()).into(),
    );
    props.insert(
        "background-color",
        <C::Property as CssProperty>::Value::new_color(255.0, 0.0, 0.0, 255.0).into(),
    );

    let id = d
        .tree
        .insert_element(NodeId::from(14u64), "div".to_string(), None, props);

    d.tree.layout_dirty_from(NodeId::from(14u64));

    let mut props = C::PropertyMap::default();

    props.insert(
        "font-size",
        <C::Property as CssProperty>::Value::new_number(16.0).into(),
    );
    props.insert(
        "color",
        <C::Property as CssProperty>::Value::new_color(0.0, 1.0, 1.0, 1.0).into(),
    );

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

pub(crate) fn test_restyle_element<B: RenderBackend, L: Layouter, D: Document<C>, C: CssSystem>(
    _d: &mut TreeDrawer<B, L, D, C>,
) {
    todo!()
}
