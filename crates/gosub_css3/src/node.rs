use core::fmt::{Display, Formatter};
use gosub_shared::byte_stream::Location;
use std::ops::Deref;

pub type Number = f32;

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureKind {
    Media,
    Container,
    Supports,
}

#[derive(Debug, PartialEq, Clone)]
pub enum NodeType {
    StyleSheet {
        children: Vec<Node>,
    },
    Rule {
        prelude: Option<Node>,
        block: Option<Node>,
    },
    AtRule {
        name: String,
        prelude: Option<Node>,
        block: Option<Node>,
    },
    Declaration {
        property: String,
        value: Vec<Node>,
        important: bool,
    },
    Block {
        children: Vec<Node>,
    },
    Comment {
        value: String,
    },
    Cdo,
    Cdc,
    IdSelector {
        value: String,
    },
    Ident {
        value: String,
    },
    Number {
        value: Number,
    },
    Percentage {
        value: Number,
    },
    Dimension {
        value: Number,
        unit: String,
    },
    Prelude,
    SelectorList {
        selectors: Vec<Node>,
    },
    AttributeSelector {
        name: String,
        matcher: Option<Node>,
        value: String,
        flags: String,
    },
    ClassSelector {
        value: String,
    },
    NestingSelector,
    TypeSelector {
        namespace: Option<String>,
        value: String,
    },
    Combinator {
        value: String,
    },
    Selector {
        children: Vec<Node>,
    },
    PseudoElementSelector {
        value: String,
    },
    PseudoClassSelector {
        value: Node,
    },
    MediaQuery {
        modifier: String,
        media_type: String,
        condition: Option<Node>,
    },
    MediaQueryList {
        media_queries: Vec<Node>,
    },
    Condition {
        list: Vec<Node>,
    },
    Feature {
        kind: FeatureKind,
        name: String,
        value: Option<Node>,
    },
    Hash {
        value: String,
    },
    Value {
        children: Vec<Node>,
    },
    Comma,
    String {
        value: String,
    },
    Url {
        url: String,
    },
    Function {
        name: String,
        arguments: Vec<Node>,
    },
    Operator(String),
    Nth {
        nth: Node,
        selector: Option<Node>,
    },
    AnPlusB {
        a: String,
        b: String,
    },
    MSFunction {
        func: Node,
    },
    MSIdent {
        value: String,
        default_value: String,
    },
    Calc {
        expr: Node,
    },
    SupportsDeclaration {
        term: Node,
    },
    FeatureFunction,
    Raw {
        value: String,
    },
    Scope {
        root: Option<Node>,
        limit: Option<Node>,
    },
    LayerList {
        layers: Vec<Node>,
    },
    ImportList {
        children: Vec<Node>,
    },
    Container {
        children: Vec<Node>,
    },
    Range {
        left: Node,
        left_comparison: Node,
        middle: Node,
        right_comparison: Option<Node>,
        right: Option<Node>,
    },
}

/// A node is a single element in the AST
#[derive(Debug, PartialEq, Clone)]
pub struct Node {
    pub node_type: Box<NodeType>,
    pub location: Location,
}

impl Node {
    pub(crate) fn new(node_type: NodeType, location: Location) -> Self {
        Self {
            node_type: Box::new(node_type),
            location,
        }
    }

    pub fn is_block(&self) -> bool {
        matches!(&*self.node_type, NodeType::Block { .. })
    }

    pub fn as_block(&self) -> &Vec<Node> {
        match &self.node_type.deref() {
            &NodeType::Block { children } => children,
            _ => panic!("Node is not a block"),
        }
    }

    pub fn is_stylesheet(&self) -> bool {
        matches!(&*self.node_type, NodeType::StyleSheet { .. })
    }

    pub fn is_rule(&self) -> bool {
        matches!(&*self.node_type, NodeType::Rule { .. })
    }

    pub fn as_stylesheet(&self) -> &Vec<Node> {
        match &self.node_type.deref() {
            &NodeType::StyleSheet { children } => children,
            _ => panic!("Node is not a stylesheet"),
        }
    }

    pub fn as_rule(&self) -> (&Option<Node>, &Option<Node>) {
        match &self.node_type.deref() {
            &NodeType::Rule { prelude, block } => (prelude, block),
            _ => panic!("Node is not a rule"),
        }
    }

    pub fn is_selector_list(&self) -> bool {
        matches!(&*self.node_type, NodeType::SelectorList { .. })
    }

    pub fn as_selector_list(&self) -> &Vec<Node> {
        match &self.node_type.deref() {
            &NodeType::SelectorList { selectors } => selectors,
            _ => panic!("Node is not a selector list"),
        }
    }

    pub fn is_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::Selector { .. })
    }

    pub fn as_selector(&self) -> &Vec<Node> {
        match &self.node_type.deref() {
            &NodeType::Selector { children } => children,
            _ => panic!("Node is not a selector"),
        }
    }

    pub fn is_ident(&self) -> bool {
        matches!(&*self.node_type, NodeType::Ident { .. })
    }

    pub fn as_ident(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::Ident { value } => value,
            _ => panic!("Node is not an ident"),
        }
    }

    pub fn is_number(&self) -> bool {
        matches!(&*self.node_type, NodeType::Number { .. })
    }

    pub fn as_number(&self) -> &Number {
        match &self.node_type.deref() {
            &NodeType::Number { value } => value,
            _ => panic!("Node is not a number"),
        }
    }

    pub fn is_hash(&self) -> bool {
        matches!(&*self.node_type, NodeType::Hash { .. })
    }

    pub fn as_hash(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::Hash { value } => value,
            _ => panic!("Node is not a hash"),
        }
    }

    pub fn as_class_selector(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::ClassSelector { value } => value,
            _ => panic!("Node is not a class selector"),
        }
    }

    pub fn is_class_selector(&self) -> bool {
        matches!(self.node_type.deref(), NodeType::ClassSelector { .. })
    }

    pub fn is_type_selector(&self) -> bool {
        match &self.node_type.deref() {
            &NodeType::TypeSelector { value, .. } => value != "*",
            _ => false,
        }
    }

    pub fn as_type_selector(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::TypeSelector { value, .. } => value,
            _ => panic!("Node is not a type selector"),
        }
    }

    pub fn is_universal_selector(&self) -> bool {
        match &self.node_type.deref() {
            &NodeType::TypeSelector { value, .. } => value == "*",
            _ => false,
        }
    }

    pub fn is_attribute_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::AttributeSelector { .. })
    }

    pub fn as_attribute_selector(&self) -> (&String, &Option<Node>, &String, &String) {
        match &self.node_type.deref() {
            &NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            } => (name, matcher, value, flags),
            _ => panic!("Node is not an attribute selector"),
        }
    }

    pub fn is_pseudo_class_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::PseudoClassSelector { .. })
    }

    pub fn as_pseudo_class_selector(&self) -> String {
        match &self.node_type.deref() {
            &NodeType::PseudoClassSelector { value } => value.to_string(),
            _ => panic!("Node is not a pseudo class selector"),
        }
    }

    pub fn is_pseudo_element_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::PseudoElementSelector { .. })
    }

    pub fn as_pseudo_element_selector(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::PseudoElementSelector { value } => value,
            _ => panic!("Node is not a pseudo element selector"),
        }
    }

    pub fn is_combinator(&self) -> bool {
        matches!(&*self.node_type, NodeType::Combinator { .. })
    }

    pub fn as_combinator(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::Combinator { value } => value,
            _ => panic!("Node is not a combinator"),
        }
    }

    pub fn is_dimension(&self) -> bool {
        matches!(self.node_type.deref(), NodeType::Dimension { .. })
    }

    pub fn as_dimension(&self) -> (&Number, &String) {
        match &self.node_type.deref() {
            &NodeType::Dimension { value, unit } => (value, unit),
            _ => panic!("Node is not a dimension"),
        }
    }

    pub fn is_id_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::IdSelector { .. })
    }

    pub fn as_id_selector(&self) -> &String {
        match &self.node_type.deref() {
            &NodeType::IdSelector { value } => value,
            _ => panic!("Node is not an id selector"),
        }
    }

    pub fn is_declaration(&self) -> bool {
        matches!(&*self.node_type, NodeType::Declaration { .. })
    }

    pub fn as_declaration(&self) -> (&String, &Vec<Node>, &bool) {
        match &self.node_type.deref() {
            &NodeType::Declaration {
                property,
                value,
                important,
            } => (property, value, important),
            _ => panic!("Node is not a declaration"),
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self.node_type.deref() {
            NodeType::SelectorList { selectors } => selectors
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .join(", "),
            NodeType::Selector { children } => children
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>()
                .join(""),
            NodeType::IdSelector { value } => value.clone(),
            NodeType::Ident { value } => value.clone(),
            NodeType::Number { value } => value.to_string(),
            NodeType::Percentage { value } => format!("{}%", value),
            NodeType::Dimension { value, unit } => format!("{}{}", value, unit),
            NodeType::Hash { value } => format!("#{}", value.clone()),
            NodeType::String { value } => value.clone(),
            NodeType::Url { url } => url.clone(),
            NodeType::Function { name, arguments } => {
                let args = arguments
                    .iter()
                    .map(|a| a.to_string())
                    .collect::<Vec<String>>()
                    .join(", ");
                format!("{}({})", name, args)
            }
            NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            } => {
                let matcher = matcher
                    .as_ref()
                    .map(|m| m.to_string())
                    .unwrap_or("".to_string());
                format!("[{}{}{}{}]", name, matcher, value, flags)
            }
            NodeType::PseudoClassSelector { value } => format!(":{}", value),
            NodeType::PseudoElementSelector { value } => format!("::{}", value),
            NodeType::Operator(value) => value.clone(),
            NodeType::ClassSelector { value } => format!(".{}", value),
            NodeType::TypeSelector { namespace, value } => {
                let ns = namespace
                    .as_ref()
                    .map(|ns| format!("{}|", ns))
                    .unwrap_or("".to_string());
                format!("{}{}", ns, value)
            }
            NodeType::Combinator { value } => value.clone(),
            NodeType::Nth { nth, selector } => {
                let sel = selector
                    .as_ref()
                    .map(|s| s.to_string())
                    .unwrap_or("".to_string());
                format!("{}{}", nth, sel)
            }
            NodeType::AnPlusB { a, b } => format!("{}n+{}", a, b),
            NodeType::Calc { expr } => format!("calc({})", expr),
            NodeType::Raw { value } => value.clone(),

            _ => {
                "".to_string()
                // panic!("cannot convert to string: {:?}", self)
            }
        };

        write!(f, "{}", s)
    }
}
