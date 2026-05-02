use crate::document::document_impl::TreeIterator;
use crate::errors::Error;
use crate::parser::query::{Condition, Query, SearchType};
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::node::NodeId;

pub struct DocumentQuery<C: HasDocument> {
    _phantom: std::marker::PhantomData<C>,
}

impl<C: HasDocument> DocumentQuery<C> {
    /// Perform a single query against the document.
    /// If query search type is uninitialized, returns an error.
    /// Otherwise, returns a vector of `NodeIds` that match the predicate in tree order (preorder depth-first.)
    pub fn query(doc: &C::Document, query: &Query) -> gosub_shared::types::Result<Vec<NodeId>> {
        if query.search_type == SearchType::Uninitialized {
            return Err(Error::Query("Query predicate is uninitialized".to_owned()).into());
        }

        let tree_iterator = TreeIterator::<C>::new(doc);

        let mut found_ids = Vec::new();
        for current_node_id in tree_iterator {
            let mut predicate_result: bool = true;
            for condition in &query.conditions {
                if !Self::matches_query_condition(doc, &current_node_id, condition) {
                    predicate_result = false;
                    break;
                }
            }

            if predicate_result {
                found_ids.push(current_node_id);
                if query.search_type == SearchType::FindFirst {
                    return Ok(found_ids);
                }
            }
        }

        Ok(found_ids)
    }

    /// Check if a given node's children contain a certain tag name
    pub fn contains_child_tag(doc: &C::Document, node_id: NodeId, tag: &str) -> bool {
        for child_id in doc.children(node_id).to_vec() {
            if doc.node_type(child_id) == NodeType::ElementNode && doc.tag_name(child_id) == Some(tag) {
                return true;
            }
        }
        false
    }

    fn matches_query_condition(doc: &C::Document, current_node_id: &NodeId, condition: &Condition) -> bool {
        if doc.node_type(*current_node_id) != NodeType::ElementNode {
            return false;
        }

        match condition {
            Condition::EqualsTag(tag) => doc.tag_name(*current_node_id) == Some(tag.as_str()),
            Condition::EqualsId(id) => doc.attribute(*current_node_id, "id") == Some(id.as_str()),
            Condition::ContainsClass(class_name) => {
                if let Some(classes) = doc.attribute(*current_node_id, "class") {
                    classes.split_whitespace().any(|c| c == class_name.as_str())
                } else {
                    false
                }
            }
            Condition::ContainsAttribute(attribute) => doc.attribute(*current_node_id, attribute).is_some(),
            Condition::ContainsChildTag(child_tag) => Self::contains_child_tag(doc, *current_node_id, child_tag),
            Condition::HasParentTag(parent_tag) => {
                if let Some(parent_id) = doc.parent(*current_node_id) {
                    doc.tag_name(parent_id) == Some(parent_tag.as_str())
                } else {
                    false
                }
            }
        }
    }
}
