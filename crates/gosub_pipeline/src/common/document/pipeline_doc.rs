use gosub_shared::node::NodeId;
use crate::common::document::document::Document;
use crate::common::document::node::NodeType;
use crate::common::document::style::{StyleProperty, StyleValue, Display};

#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNodeKind {
    Text,
    Comment,
    Element,
}

pub trait PipelineDocument: Send + Sync {
    fn root(&self) -> Option<NodeId>;
    fn children(&self, id: NodeId) -> Vec<NodeId>;
    fn node_kind(&self, id: NodeId) -> PipelineNodeKind;
    fn tag_name(&self, id: NodeId) -> Option<String>;
    fn is_display_none(&self, id: NodeId) -> bool;
    fn parent(&self, id: NodeId) -> Option<NodeId>;
    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue>;
    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32;
    fn html_node_id(&self) -> Option<NodeId>;
    fn body_node_id(&self) -> Option<NodeId>;
    fn base_url(&self) -> String;
    fn inner_html(&self, id: NodeId) -> String;
}

// impl for poc's own Document (field-based, self-contained)
impl PipelineDocument for Document {
    fn root(&self) -> Option<NodeId> {
        self.root_id
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.arena.get(&id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        match self.arena.get(&id) {
            Some(node) => match &node.node_type {
                NodeType::Text(..) => PipelineNodeKind::Text,
                NodeType::Comment(..) => PipelineNodeKind::Comment,
                NodeType::Element(..) => PipelineNodeKind::Element,
            },
            None => PipelineNodeKind::Comment,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.arena.get(&id).and_then(|node| match &node.node_type {
            NodeType::Element(data) => Some(data.tag_name.clone()),
            _ => None,
        })
    }

    fn is_display_none(&self, id: NodeId) -> bool {
        self.arena.get(&id).map(|node| match &node.node_type {
            NodeType::Element(data) => matches!(
                data.get_style(StyleProperty::Display),
                Some(StyleValue::Display(d)) if *d == Display::None
            ),
            _ => false,
        }).unwrap_or(false)
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.arena.get(&id).and_then(|n| n.parent_id)
    }

    fn get_style(&self, id: NodeId, prop: StyleProperty) -> Option<StyleValue> {
        self.arena.get(&id).and_then(|node| match &node.node_type {
            NodeType::Element(data) => data.get_style(prop).cloned(),
            _ => None,
        })
    }

    fn get_style_f32(&self, id: NodeId, prop: StyleProperty) -> f32 {
        match self.get_style(id, prop) {
            Some(StyleValue::Unit(px, _)) => px,
            Some(StyleValue::Number(px)) => px,
            _ => 0.0,
        }
    }

    fn html_node_id(&self) -> Option<NodeId> {
        self.html_node_id
    }

    fn body_node_id(&self) -> Option<NodeId> {
        self.body_node_id
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn inner_html(&self, id: NodeId) -> String {
        self.inner_html(id)
    }
}

/// Adapter that wraps any `gosub_interface::document::Document<C>` and exposes it
/// as a `PipelineDocument`. CSS display:none filtering is deferred to Step 3.
pub struct GosubDocumentAdapter<C, D>
where
    C: gosub_interface::config::HasCssSystem,
    D: gosub_interface::document::Document<C>,
{
    pub doc: D,
    _phantom: std::marker::PhantomData<C>,
}

impl<C, D> GosubDocumentAdapter<C, D>
where
    C: gosub_interface::config::HasCssSystem,
    D: gosub_interface::document::Document<C>,
{
    pub fn new(doc: D) -> Self {
        Self { doc, _phantom: std::marker::PhantomData }
    }
}

impl<C, D> PipelineDocument for GosubDocumentAdapter<C, D>
where
    C: gosub_interface::config::HasCssSystem + Send + Sync + 'static,
    D: gosub_interface::document::Document<C> + Send + Sync,
{
    fn root(&self) -> Option<NodeId> {
        Some(self.doc.root())
    }

    fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.doc.children(id).to_vec()
    }

    fn node_kind(&self, id: NodeId) -> PipelineNodeKind {
        use gosub_interface::node::NodeType as GosubNodeType;
        match self.doc.node_type(id) {
            GosubNodeType::TextNode => PipelineNodeKind::Text,
            GosubNodeType::CommentNode => PipelineNodeKind::Comment,
            GosubNodeType::ElementNode => PipelineNodeKind::Element,
            _ => PipelineNodeKind::Comment,
        }
    }

    fn tag_name(&self, id: NodeId) -> Option<String> {
        self.doc.tag_name(id).map(|s| s.to_string())
    }

    fn is_display_none(&self, _id: NodeId) -> bool {
        // CSS integration deferred to Step 3
        false
    }

    fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.doc.parent(id)
    }

    fn get_style(&self, _id: NodeId, _prop: StyleProperty) -> Option<StyleValue> {
        // CSS integration deferred to Step 3
        None
    }

    fn get_style_f32(&self, _id: NodeId, _prop: StyleProperty) -> f32 {
        // CSS integration deferred to Step 3
        0.0
    }

    fn html_node_id(&self) -> Option<NodeId> {
        // gosub_interface doesn't track html/body by name; Step 3 will resolve this
        None
    }

    fn body_node_id(&self) -> Option<NodeId> {
        None
    }

    fn base_url(&self) -> String {
        self.doc.url().map(|u| u.to_string()).unwrap_or_default()
    }

    fn inner_html(&self, id: NodeId) -> String {
        self.doc.write_from_node(id)
    }
}
