//! Element scopes: whether an element is in scope on the stack of open elements.

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

/// Defines the scopes for `in_scope()`
#[derive(Clone, Copy)]
pub(crate) enum Scope {
    Regular,
    ListItem,
    Button,
    Table,
    Select,
}

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Checks if the given element is in given scope
    pub(crate) fn is_in_scope(&self, tag: &str, namespace: &str, scope: Scope) -> bool {
        // Asked before every block start tag, so it is cached. See `OpenElements`.
        if matches!(scope, Scope::Button) && tag == "p" && namespace == HTML_NAMESPACE {
            return self
                .open_elements
                .has_p_in_button_scope(|node_id| self.scope_step(node_id, tag, namespace, scope));
        }
        self.open_elements
            .iter()
            .rev()
            .find_map(|&node_id| self.scope_step(node_id, tag, namespace, scope))
            .unwrap_or(false)
    }

    /// Whether `target` itself is in `scope`. Unlike `is_in_scope`, another element with the
    /// same name above it does not count.
    pub(crate) fn is_element_in_scope(&self, target: NodeId, scope: Scope) -> bool {
        for &node_id in self.open_elements.iter().rev() {
            if node_id == target {
                return true;
            }
            if self.ends_scope(node_id, scope) {
                return false;
            }
        }
        false
    }

    /// One step of the scope walk. `Some(true)` when `node_id` is the element looked for,
    /// `Some(false)` when it ends the scope, `None` to go on to the next entry down.
    pub(crate) fn scope_step(&self, node_id: NodeId, tag: &str, namespace: &str, scope: Scope) -> Option<bool> {
        if self.document.node_type(node_id) == NodeType::ElementNode
            && self.document.tag_name(node_id).unwrap_or_default() == tag
            && self.document.namespace(node_id).unwrap_or(HTML_NAMESPACE) == namespace
        {
            return Some(true);
        }
        self.ends_scope(node_id, scope).then_some(false)
    }

    /// Whether `node_id` is one of the elements that end `scope`.
    pub(crate) fn ends_scope(&self, node_id: NodeId, scope: Scope) -> bool {
        if self.document.node_type(node_id) != NodeType::ElementNode {
            return true;
        }

        let node_tag = self.document.tag_name(node_id).unwrap_or_default();
        let node_ns = self.document.namespace(node_id).unwrap_or(HTML_NAMESPACE);

        let default_html_scope = [
            "applet", "caption", "html", "table", "td", "th", "marquee", "object", "template",
        ];
        let default_mathml_scope = ["mo", "mi", "ms", "mn", "mtext", "annotation-xml"];
        let default_svg_scope = ["foreignObject", "desc", "title"];
        match scope {
            Scope::Regular => {
                if (node_ns == HTML_NAMESPACE && default_html_scope.contains(&node_tag))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::ListItem => {
                if (node_ns == HTML_NAMESPACE
                    && (default_html_scope.contains(&node_tag) || ["ol", "ul"].contains(&node_tag)))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::Button => {
                if (node_ns == HTML_NAMESPACE && (default_html_scope.contains(&node_tag) || node_tag == "button"))
                    || (node_ns == MATHML_NAMESPACE && default_mathml_scope.contains(&node_tag))
                    || (node_ns == SVG_NAMESPACE && default_svg_scope.contains(&node_tag))
                {
                    return true;
                }
            }
            Scope::Table => {
                if node_ns == HTML_NAMESPACE && ["html", "template", "table"].contains(&node_tag) {
                    return true;
                }
            }
            Scope::Select => {
                if !(node_ns == HTML_NAMESPACE && ["optgroup", "option"].contains(&node_tag)) {
                    return true;
                }
            }
        }
        false
    }
}
