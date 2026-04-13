use crate::config::HasDocument;
use derive_more::Display;
use std::collections::hash_map::IntoIter;
use std::collections::HashMap;
use std::fmt;
use std::fmt::{Debug, Display, Formatter};

/// Location holds the start position of the given element in the data source
#[derive(Clone, PartialEq, Copy)]
pub struct Location {
    /// Line number, starting with 1
    pub line: usize,
    /// Column number, starting with 1
    pub column: usize,
    /// Byte offset, starting with 0
    pub offset: usize,
}

impl Default for Location {
    /// Default to line 1, column 1
    fn default() -> Self {
        Self::new(1, 1, 0)
    }
}

impl Location {
    /// Create a new Location
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self { line, column, offset }
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.line, self.column)
    }
}

impl Debug for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.line, self.column)
    }
}

/// A NodeID is a unique identifier for a node in a node tree.
#[derive(Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, PartialOrd)]
pub struct NodeId(usize);

impl From<NodeId> for usize {
    /// Converts a NodeId into a usize
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl From<usize> for NodeId {
    /// Converts a usize into a NodeId
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<u64> for NodeId {
    /// Converts a u64 into a NodeId
    fn from(value: u64) -> Self {
        Self(value as usize)
    }
}

impl From<NodeId> for u64 {
    /// Converts a NodeId into a u64
    fn from(value: NodeId) -> Self {
        value.0 as u64
    }
}

impl Default for &NodeId {
    /// Returns the default NodeId, which is 0
    fn default() -> Self {
        &NodeId(0)
    }
}

impl NodeId {
    // TODO: Drop Default derive and only use 0 for the root, or choose another id for the root
    pub const ROOT_NODE: usize = 0;

    /// Returns the root node ID
    pub fn root() -> Self {
        Self(Self::ROOT_NODE)
    }

    /// Returns true when this nodeId is the root node
    pub fn is_root(&self) -> bool {
        self.0 == Self::ROOT_NODE
    }

    /// Returns the next node ID
    #[must_use]
    pub fn next(&self) -> Self {
        if self.0 == usize::MAX {
            return Self(usize::MAX);
        }

        Self(self.0 + 1)
    }

    /// Returns the nodeID as usize
    pub fn as_usize(&self) -> usize {
        self.0
    }

    /// Returns the previous node ID
    #[must_use]
    pub fn prev(&self) -> Self {
        if self.0 == 0 {
            return Self::root();
        }

        Self(self.0 - 1)
    }
}

#[derive(PartialEq, Debug, Copy, Clone)]
pub enum QuirksMode {
    Quirks,
    LimitedQuirks,
    NoQuirks,
}

/// Different types of nodes that all have their own data structures (`NodeData`)
#[derive(Debug, PartialEq)]
pub enum NodeType {
    DocumentNode,
    DocTypeNode,
    TextNode,
    CommentNode,
    ElementNode,
}

/// Different types of nodes
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum NodeData<'a, C: HasDocument> {
    Document(&'a C::DocumentData),
    DocType(&'a C::DocTypeData),
    Text(&'a C::TextData),
    Comment(&'a C::CommentData),
    Element(&'a C::ElementData),
}

// impl<C: HasDocument> Copy for NodeData<'_, C> {}
//
// impl<C: HasDocument> Clone for NodeData<'_, C> {
//     fn clone(&self) -> Self {
//         *self
//     }
// }

pub trait DocumentDataType {
    fn quirks_mode(&self) -> QuirksMode;
    fn set_quirks_mode(&mut self, quirks_mode: QuirksMode);
}

pub trait DocTypeDataType {
    fn name(&self) -> &str;
    fn pub_identifier(&self) -> &str;
    fn sys_identifier(&self) -> &str;
}

pub trait TextDataType {
    fn value(&self) -> &str;

    fn string_value(&self) -> String;
    fn value_mut(&mut self) -> &mut String;
}

pub trait CommentDataType {
    fn value(&self) -> &str;
}

pub trait ClassList {
    /// Returns true when the classlist contains the given class name
    fn contains(&self, class_name: &str) -> bool;
    /// Adds a class to the classlist
    fn add(&mut self, class_name: &str);
    /// Removes a class from the classlist
    fn remove(&mut self, class_name: &str);
    /// Toggles a class active/inactive in the classlist
    fn toggle(&mut self, class_name: &str);
    /// Replaces a class in the classlist
    fn replace(&mut self, old_class_name: &str, new_class_name: &str);
    /// Returns the number of classes in the classlist
    fn length(&self) -> usize;
    /// Returns the classes as a vector
    fn as_vec(&self) -> Vec<String>;
    /// Returns true if the classlist is empty
    fn is_active(&self, class_name: &str) -> bool;
    /// Returns the active classes of the classlist
    fn active_classes(&self) -> Vec<String>;
    /// Returns the active classes of the classlist as a string
    fn len(&self) -> usize;
    /// Returns true if the classlist is empty
    fn is_empty(&self) -> bool;
    /// Sets the active state of a class
    fn set_active(&mut self, name: &str, is_active: bool);
    fn iter(&self) -> IntoIter<String, bool>;
}

pub trait ElementDataType<C: HasDocument> {
    /// Returns the name of the element
    fn name(&self) -> &str;

    /// Returns the namespace
    fn namespace(&self) -> &str;
    /// Returns true if the namespace matches the element
    fn is_namespace(&self, namespace: &str) -> bool;

    /// Returns the classes of the element
    fn classlist(&self) -> &impl ClassList;
    fn classlist_mut(&mut self) -> &mut impl ClassList;
    /// Returns the active classes of the element
    fn active_class_names(&self) -> Vec<String>;

    /// Returns the given attribute (or None when not found)
    fn attribute(&self, name: &str) -> Option<&String>;
    /// Returns all attributes of the element
    fn attributes(&self) -> &HashMap<String, String>;
    /// Add attribute
    fn add_attribute(&mut self, name: &str, value: &str);
    /// Remove an attribute
    fn remove_attribute(&mut self, name: &str);
    /// Add a class to the element
    fn add_class(&mut self, class: &str);

    fn matches_tag_and_attrs_without_order(&self, other_data: &Self) -> bool;
    fn is_mathml_integration_point(&self) -> bool;
    fn is_html_integration_point(&self) -> bool;

    /// Returns true if this is a "special" element node
    fn is_special(&self) -> bool;

    // Return the template document of the element
    fn template_contents(&self) -> Option<&C::DocumentFragment>;
    /// Returns true if the given node is a "formatting" node
    fn is_formatting(&self) -> bool;

    fn set_template_contents(&mut self, template_contents: C::DocumentFragment);
}

pub trait Node<C: HasDocument>: Clone + Debug + PartialEq {
    type DocumentData: DocumentDataType;
    type DocTypeData: DocTypeDataType;
    type TextData: TextDataType;
    type CommentData: CommentDataType;
    type ElementData: ElementDataType<C>;

    fn new_from_node(org_node: &Self) -> Self;

    /// Return the ID of the node
    fn id(&self) -> NodeId;
    /// Sets the ID of the node
    fn set_id(&mut self, id: NodeId);
    /// Returns the location of the node
    fn location(&self) -> Location;
    /// Returns the ID of the parent node or None when the node is the root
    fn parent_id(&self) -> Option<NodeId>;
    /// Sets the parent of the node, or None when the node is the root
    fn set_parent(&mut self, parent_id: Option<NodeId>);

    fn set_registered(&mut self, registered: bool);
    fn is_registered(&self) -> bool;

    /// Returns true when this node is the root node
    fn is_root(&self) -> bool;
    /// Returns the children of the node
    fn children(&self) -> &[NodeId];

    /// Returns the type of the node
    fn type_of(&self) -> NodeType;

    fn is_element_node(&self) -> bool;
    fn get_element_data(&self) -> Option<&Self::ElementData>;
    fn get_element_data_mut(&mut self) -> Option<&mut Self::ElementData>;

    fn is_text_node(&self) -> bool;
    fn get_text_data(&self) -> Option<&Self::TextData>;
    fn get_text_data_mut(&mut self) -> Option<&mut Self::TextData>;

    fn get_comment_data(&self) -> Option<&Self::CommentData>;
    fn get_doctype_data(&self) -> Option<&Self::DocTypeData>;

    /// Removes a child node from the node
    fn remove(&mut self, node_id: NodeId);
    /// Inserts a child node to the node at a specific index
    fn insert(&mut self, node_id: NodeId, idx: usize);
    /// Pushes a child node to the node
    fn push(&mut self, node_id: NodeId);

    fn data(&self) -> NodeData<'_, C>;
}
