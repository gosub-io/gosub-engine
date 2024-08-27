use crate::render_tree::{RenderNodeData, RenderTree};
use gosub_html5::node::NodeId;
use gosub_render_backend::layout::{Layout, Layouter};
use gosub_render_backend::{NodeDesc, Point, PreRenderText, RenderBackend, Size};

impl<B: RenderBackend, L: Layouter> RenderTree<B, L> {
    pub fn desc(&self) -> NodeDesc {
        self.desc_node(self.root)
    }

    fn desc_node(&self, node: NodeId) -> NodeDesc {
        let Some(node) = self.get_node(node) else {
            return NodeDesc {
                id: 0,
                name: "<unknown>".into(),
                children: vec![],
                attributes: vec![],
                properties: vec![],
                text: None,
                pos: Point::ZERO,
                size: Size::ZERO,
            };
        };

        let attributes = if let RenderNodeData::Element(e) = &node.data {
            e.attributes
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else {
            vec![]
        };

        let (name, text) = if let RenderNodeData::Text(t) = &node.data {
            ("#text".into(), Some(t.prerender.value().into()))
        } else {
            (node.name.clone(), None)
        };

        NodeDesc {
            id: node.id.into(),
            name,
            children: node
                .children
                .iter()
                .map(|child| self.desc_node(*child))
                .collect(),
            attributes,
            properties: node
                .properties
                .properties
                .iter()
                .map(|(k, v)| (k.clone(), v.actual.to_string()))
                .collect(),
            text,
            pos: node.layout.rel_pos(),
            size: node.layout.size(),
        }
    }
}
