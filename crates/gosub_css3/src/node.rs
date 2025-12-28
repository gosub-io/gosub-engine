use core::fmt::{Display, Formatter};
use gosub_shared::byte_stream::Location;

pub type Number = f32;

#[derive(Debug, Clone, PartialEq, Eq)]
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

    #[must_use]
    pub fn is_block(&self) -> bool {
        matches!(&*self.node_type, NodeType::Block { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Block`.
    pub fn as_block(&self) -> &Vec<Self> {
        match &&*self.node_type {
            &NodeType::Block { children } => children,
            _ => panic!("Node is not a block"),
        }
    }

    #[must_use]
    pub fn is_stylesheet(&self) -> bool {
        matches!(&*self.node_type, NodeType::StyleSheet { .. })
    }

    #[must_use]
    pub fn is_rule(&self) -> bool {
        matches!(&*self.node_type, NodeType::Rule { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `StyleSheet`.
    pub fn as_stylesheet(&self) -> &Vec<Self> {
        match &&*self.node_type {
            &NodeType::StyleSheet { children } => children,
            _ => panic!("Node is not a stylesheet"),
        }
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Rule`.
    pub fn as_rule(&self) -> (&Option<Self>, &Option<Self>) {
        match &&*self.node_type {
            &NodeType::Rule { prelude, block } => (prelude, block),
            _ => panic!("Node is not a rule"),
        }
    }

    #[must_use]
    pub fn is_selector_list(&self) -> bool {
        matches!(&*self.node_type, NodeType::SelectorList { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `SelectorList`.
    pub fn as_selector_list(&self) -> &Vec<Self> {
        match &&*self.node_type {
            &NodeType::SelectorList { selectors } => selectors,
            _ => panic!("Node is not a selector list"),
        }
    }

    #[must_use]
    pub fn is_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::Selector { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Selector`.
    pub fn as_selector(&self) -> &Vec<Self> {
        match &&*self.node_type {
            &NodeType::Selector { children } => children,
            _ => panic!("Node is not a selector"),
        }
    }

    #[must_use]
    pub fn is_ident(&self) -> bool {
        matches!(&*self.node_type, NodeType::Ident { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Ident`.
    pub fn as_ident(&self) -> &String {
        match &&*self.node_type {
            &NodeType::Ident { value } => value,
            _ => panic!("Node is not an ident"),
        }
    }

    #[must_use]
    pub fn is_number(&self) -> bool {
        matches!(&*self.node_type, NodeType::Number { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Number`.
    pub fn as_number(&self) -> &Number {
        match &&*self.node_type {
            &NodeType::Number { value } => value,
            _ => panic!("Node is not a number"),
        }
    }

    #[must_use]
    pub fn is_hash(&self) -> bool {
        matches!(&*self.node_type, NodeType::Hash { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Hash`.
    pub fn as_hash(&self) -> &String {
        match &&*self.node_type {
            &NodeType::Hash { value } => value,
            _ => panic!("Node is not a hash"),
        }
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `ClassSelector`.
    pub fn as_class_selector(&self) -> &String {
        match &&*self.node_type {
            &NodeType::ClassSelector { value } => value,
            _ => panic!("Node is not a class selector"),
        }
    }

    #[must_use]
    pub fn is_class_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::ClassSelector { .. })
    }

    #[must_use]
    pub fn is_type_selector(&self) -> bool {
        match &&*self.node_type {
            &NodeType::TypeSelector { value, .. } => value != "*",
            _ => false,
        }
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `TypeSelector`.
    pub fn as_type_selector(&self) -> &String {
        match &&*self.node_type {
            &NodeType::TypeSelector { value, .. } => value,
            _ => panic!("Node is not a type selector"),
        }
    }

    #[must_use]
    pub fn is_universal_selector(&self) -> bool {
        match &&*self.node_type {
            &NodeType::TypeSelector { value, .. } => value == "*",
            _ => false,
        }
    }

    #[must_use]
    pub fn is_attribute_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::AttributeSelector { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `AttributeSelector`.
    pub fn as_attribute_selector(&self) -> (&String, &Option<Self>, &String, &String) {
        match &&*self.node_type {
            &NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            } => (name, matcher, value, flags),
            _ => panic!("Node is not an attribute selector"),
        }
    }

    #[must_use]
    pub fn is_pseudo_class_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::PseudoClassSelector { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `PseudoClassSelector`.
    pub fn as_pseudo_class_selector(&self) -> String {
        match &&*self.node_type {
            &NodeType::PseudoClassSelector { value } => value.to_string(),
            _ => panic!("Node is not a pseudo class selector"),
        }
    }

    #[must_use]
    pub fn is_pseudo_element_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::PseudoElementSelector { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `PseudoElementSelector`.
    pub fn as_pseudo_element_selector(&self) -> &String {
        match &&*self.node_type {
            &NodeType::PseudoElementSelector { value } => value,
            _ => panic!("Node is not a pseudo element selector"),
        }
    }

    #[must_use]
    pub fn is_combinator(&self) -> bool {
        matches!(&*self.node_type, NodeType::Combinator { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Combinator`.
    pub fn as_combinator(&self) -> &String {
        match &&*self.node_type {
            &NodeType::Combinator { value } => value,
            _ => panic!("Node is not a combinator"),
        }
    }

    #[must_use]
    pub fn is_dimension(&self) -> bool {
        matches!(&*self.node_type, NodeType::Dimension { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Dimension`.
    pub fn as_dimension(&self) -> (&Number, &String) {
        match &&*self.node_type {
            &NodeType::Dimension { value, unit } => (value, unit),
            _ => panic!("Node is not a dimension"),
        }
    }

    #[must_use]
    pub fn is_id_selector(&self) -> bool {
        matches!(&*self.node_type, NodeType::IdSelector { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `IdSelector`.
    pub fn as_id_selector(&self) -> &String {
        match &&*self.node_type {
            &NodeType::IdSelector { value } => value,
            _ => panic!("Node is not an id selector"),
        }
    }

    #[must_use]
    pub fn is_declaration(&self) -> bool {
        matches!(&*self.node_type, NodeType::Declaration { .. })
    }

    #[must_use]
    /// # Panics
    ///
    /// Panics if the node is not of type `Declaration`.
    pub fn as_declaration(&self) -> (&String, &Vec<Self>, &bool) {
        match &&*self.node_type {
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
        let s = match &*self.node_type {
            NodeType::SelectorList { selectors } => selectors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<String>>()
                .join(", "),
            NodeType::Selector { children } => children
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<String>(),
            NodeType::IdSelector { value }
            | NodeType::Ident { value }
            | NodeType::String { value }
            | NodeType::Operator(value)
            | NodeType::Combinator { value }
            | NodeType::Raw { value } => value.clone(),
            NodeType::Number { value } => value.to_string(),
            NodeType::Percentage { value } => format!("{value}%"),
            NodeType::Dimension { value, unit } => format!("{value}{unit}"),
            NodeType::Hash { value } => format!("#{value}"),
            NodeType::Url { url } => url.clone(),
            NodeType::Function { name, arguments } => {
                let args = arguments
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<String>>()
                    .join(", ");
                format!("{name}({args})")
            }
            NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            } => {
                let matcher = matcher.as_ref().map_or(String::new(), std::string::ToString::to_string);
                format!("[{name}{matcher}{value}{flags}]")
            }
            NodeType::PseudoClassSelector { value } => format!(":{value}"),
            NodeType::PseudoElementSelector { value } => format!("::{value}"),
            NodeType::ClassSelector { value } => format!(".{value}"),
            NodeType::TypeSelector { namespace, value } => {
                let ns = namespace.as_ref().map_or(String::new(), |ns| format!("{ns}|"));
                format!("{ns}{value}")
            }
            NodeType::Nth { nth, selector } => {
                let sel = selector
                    .as_ref()
                    .map_or(String::new(), std::string::ToString::to_string);
                format!("{nth}{sel}")
            }
            NodeType::AnPlusB { a, b } => format!("{a}n+{b}"),
            NodeType::Calc { expr } => format!("calc({expr})"),

            _ => {
                String::new()
                // panic!("cannot convert to string: {:?}", self)
            }
        };

        write!(f, "{s}")
    }
}
