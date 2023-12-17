use crate::css3::location::Location;

pub type Number = f32;

#[derive(Debug, Clone, PartialEq)]
pub enum FeatureKind {
    Media,
    Container,
}

#[derive(Debug, PartialEq)]
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
        value: String
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
        arguments: Vec<Node>
    },
    Operator(String),
    Nth { nth: Node, selector: Option<Node> },
    AnPlusB { a: String, b: String },
    OpacityIE8Hack { value: Number },
}

/// A node is a single element in the AST
#[derive(Debug, PartialEq)]
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
}
