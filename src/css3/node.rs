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
    TypeSelector,
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
}

/// A node is a single element in the AST
#[derive(Debug, PartialEq)]
pub struct Node {
    pub node_type: Box<NodeType>,
}

impl Node {
    pub(crate) fn new_media_query_list(queries: Vec<Node>) -> Node {
        Self {
            node_type: Box::new(NodeType::MediaQueryList {
                media_queries: queries,
            }),
        }
    }

    pub(crate) fn new_media_query(
        modifier: String,
        media_type: String,
        condition: Option<Node>,
    ) -> Node {
        Self {
            node_type: Box::new(NodeType::MediaQuery {
                modifier,
                media_type,
                condition,
            }),
        }
    }

    pub(crate) fn new_selector(children: Vec<Node>) -> Node {
        Self {
            node_type: Box::new(NodeType::Selector { children }),
        }
    }

    pub(crate) fn new_combinator(name: String) -> Node {
        Self {
            node_type: Box::new(NodeType::Combinator { value: name }),
        }
    }

    pub(crate) fn new_attribute_selector(
        name: String,
        matcher: Option<Node>,
        value: String,
        flags: String,
    ) -> Node {
        Self {
            node_type: Box::new(NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            }),
        }
    }
}

impl Node {
    pub(crate) fn new(node_type: NodeType) -> Self {
        Self {
            node_type: Box::new(node_type),
        }
    }

    pub fn new_stylesheet(children: Vec<Node>) -> Self {
        Self {
            node_type: Box::new(NodeType::StyleSheet { children }),
        }
    }

    pub fn new_rule(prelude: Node, block: Node) -> Self {
        Self {
            node_type: Box::new(NodeType::Rule {
                prelude: Some(prelude),
                block: Some(block),
            }),
        }
    }

    pub fn new_at_rule(name: String, prelude: Option<Node>, block: Option<Node>) -> Self {
        Self {
            node_type: Box::new(NodeType::AtRule {
                name,
                prelude,
                block,
            }),
        }
    }

    pub fn new_declaration(property: String, value: Vec<Node>, important: bool) -> Self {
        Self {
            node_type: Box::new(NodeType::Declaration {
                property,
                value,
                important,
            }),
        }
    }

    pub(crate) fn new_comment(s: String) -> Node {
        Self {
            node_type: Box::new(NodeType::Comment { value: s }),
        }
    }

    pub(crate) fn new_cdo() -> Node {
        Self {
            node_type: Box::new(NodeType::Cdo),
        }
    }

    pub(crate) fn new_cdc() -> Node {
        Self {
            node_type: Box::new(NodeType::Cdc),
        }
    }

    pub(crate) fn new_selector_list(selectors: Vec<Node>) -> Node {
        Self {
            node_type: Box::new(NodeType::SelectorList { selectors }),
        }
    }
}
