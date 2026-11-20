use crate::tokenizer::NumberKind;
use compact_str::CompactString;
use core::fmt::{Display, Formatter};
use gosub_shared::byte_stream::Location;

pub type Number = f64;

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
        prelude: Option<Box<Node>>,
        block: Option<Box<Node>>,
    },
    AtRule {
        name: CompactString,
        prelude: Option<Box<Node>>,
        block: Option<Box<Node>>,
    },
    Declaration {
        property: CompactString,
        value: Vec<Node>,
        important: bool,
    },
    Block {
        children: Vec<Node>,
    },
    Comment {
        value: CompactString,
    },
    Cdo,
    Cdc,
    IdSelector {
        value: CompactString,
    },
    Ident {
        value: CompactString,
    },
    Number {
        value: Number,
        /// Whether it was written as an integer. `<integer>` needs the spelling, not the value.
        kind: NumberKind,
    },
    Percentage {
        value: Number,
    },
    Dimension {
        value: Number,
        unit: CompactString,
    },
    Prelude,
    SelectorList {
        selectors: Vec<Node>,
    },
    AttributeSelector {
        name: CompactString,
        matcher: Option<Box<Node>>,
        value: CompactString,
        flags: CompactString,
    },
    ClassSelector {
        value: CompactString,
    },
    NestingSelector,
    TypeSelector {
        namespace: Option<CompactString>,
        value: CompactString,
    },
    Combinator {
        value: CompactString,
    },
    Selector {
        children: Vec<Node>,
    },
    PseudoElementSelector {
        value: CompactString,
        /// Selector list of a functional pseudo-element, kept only for `::slotted()`.
        arguments: Option<Box<Node>>,
    },
    PseudoClassSelector {
        value: Box<Node>,
    },
    MediaQuery {
        modifier: CompactString,
        media_type: CompactString,
        condition: Option<Box<Node>>,
    },
    MediaQueryList {
        media_queries: Vec<Node>,
    },
    Condition {
        list: Vec<Node>,
    },
    Feature {
        kind: FeatureKind,
        name: CompactString,
        value: Option<Box<Node>>,
    },
    Hash {
        value: CompactString,
    },
    Value {
        children: Vec<Node>,
    },
    Comma,
    String {
        value: CompactString,
    },
    Url {
        url: CompactString,
    },
    Function {
        name: CompactString,
        arguments: Vec<Node>,
    },
    /// An operator token in a value: `+`, `-`, `*`, `/`, `,`, `:`, `=`.
    ///
    /// The surrounding whitespace is part of the token because css-values-4 §10.1 makes it
    /// load-bearing: `+` and `-` require whitespace on *both* sides, which is the only thing
    /// separating `calc(1px - 2px)` (a subtraction) from `calc(1px -2px)` (two adjacent
    /// values). The generic value parser discards whitespace, so without recording it here the
    /// rule is unenforceable for every math function but `calc()`.
    ///
    /// Both flags are false outside a value sequence (selectors, media queries), where no rule
    /// depends on them.
    Operator {
        value: CompactString,
        space_before: bool,
        space_after: bool,
    },
    Nth {
        nth: Box<Node>,
        selector: Option<Box<Node>>,
    },
    AnPlusB {
        a: CompactString,
        b: CompactString,
    },
    MSFunction {
        func: Box<Node>,
    },
    MSIdent {
        value: CompactString,
        default_value: CompactString,
    },
    /// A `calc()` body, as the tokens that make it up: values, operators with their
    /// whitespace, nested functions, in source order. A parenthesized group is a nested
    /// `calc()`, which css-values-4 says it is.
    ///
    /// This used to be a single [`NodeType::Raw`] holding the body re-serialized back into
    /// text, which the evaluator then tokenized for a second time. The round trip was lossy in
    /// exactly the place it could least afford to be - `calc(1px +2px)` came back as
    /// `calc(1px 2px)`, because the tokenizer folds a leading `+` into the number and a
    /// dimension prints without its sign.
    Calc {
        tokens: Vec<Node>,
    },
    SupportsDeclaration {
        term: Box<Node>,
    },
    FeatureFunction,
    Raw {
        value: CompactString,
    },
    Scope {
        root: Option<Box<Node>>,
        limit: Option<Box<Node>>,
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
        left: Box<Node>,
        left_comparison: Box<Node>,
        middle: Box<Node>,
        right_comparison: Option<Box<Node>>,
        right: Option<Box<Node>>,
    },
}

impl NodeType {
    /// An operator with no recorded whitespace, which is the right default everywhere the
    /// surrounding whitespace carries no meaning. `parse_value_sequence` fills the flags in for
    /// the one context that needs them.
    pub(crate) fn operator(value: impl Into<CompactString>) -> Self {
        NodeType::Operator {
            value: value.into(),
            space_before: false,
            space_after: false,
        }
    }
}

/// A node is a single element in the AST
#[derive(Debug, PartialEq, Clone)]
pub struct Node {
    pub node_type: NodeType,
    pub location: Location,
}

impl Node {
    pub(crate) fn new(node_type: NodeType, location: Location) -> Self {
        Self { node_type, location }
    }

    #[must_use]
    pub fn is_block(&self) -> bool {
        matches!(&self.node_type, NodeType::Block { .. })
    }

    #[must_use]
    pub fn as_block(&self) -> Option<&Vec<Node>> {
        match &self.node_type {
            NodeType::Block { children } => Some(children),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_stylesheet(&self) -> bool {
        matches!(&self.node_type, NodeType::StyleSheet { .. })
    }

    #[must_use]
    pub fn is_rule(&self) -> bool {
        matches!(&self.node_type, NodeType::Rule { .. })
    }

    #[must_use]
    pub fn as_stylesheet(&self) -> Option<&Vec<Node>> {
        match &self.node_type {
            NodeType::StyleSheet { children } => Some(children),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_rule(&self) -> Option<(Option<&Node>, Option<&Node>)> {
        match &self.node_type {
            NodeType::Rule { prelude, block } => Some((prelude.as_deref(), block.as_deref())),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_selector_list(&self) -> bool {
        matches!(&self.node_type, NodeType::SelectorList { .. })
    }

    #[must_use]
    pub fn as_selector_list(&self) -> Option<&Vec<Node>> {
        match &self.node_type {
            NodeType::SelectorList { selectors } => Some(selectors),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::Selector { .. })
    }

    #[must_use]
    pub fn as_selector(&self) -> Option<&Vec<Node>> {
        match &self.node_type {
            NodeType::Selector { children } => Some(children),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_ident(&self) -> bool {
        matches!(&self.node_type, NodeType::Ident { .. })
    }

    #[must_use]
    pub fn as_ident(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::Ident { value } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_number(&self) -> bool {
        matches!(&self.node_type, NodeType::Number { .. })
    }

    #[must_use]
    pub fn as_number(&self) -> Option<&Number> {
        match &self.node_type {
            NodeType::Number { value, .. } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_hash(&self) -> bool {
        matches!(&self.node_type, NodeType::Hash { .. })
    }

    #[must_use]
    pub fn as_hash(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::Hash { value } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_class_selector(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::ClassSelector { value } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_class_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::ClassSelector { .. })
    }

    #[must_use]
    pub fn is_type_selector(&self) -> bool {
        match &self.node_type {
            NodeType::TypeSelector { value, .. } => value != "*",
            _ => false,
        }
    }

    #[must_use]
    pub fn as_type_selector(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::TypeSelector { value, .. } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_universal_selector(&self) -> bool {
        match &self.node_type {
            NodeType::TypeSelector { value, .. } => value == "*",
            _ => false,
        }
    }

    #[must_use]
    pub fn is_attribute_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::AttributeSelector { .. })
    }

    #[must_use]
    pub fn as_attribute_selector(&self) -> Option<(&CompactString, Option<&Node>, &CompactString, &CompactString)> {
        match &self.node_type {
            NodeType::AttributeSelector {
                name,
                matcher,
                value,
                flags,
            } => Some((name, matcher.as_deref(), value, flags)),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_pseudo_class_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::PseudoClassSelector { .. })
    }

    #[must_use]
    pub fn as_pseudo_class_selector(&self) -> Option<String> {
        match &self.node_type {
            NodeType::PseudoClassSelector { value } => Some(value.to_string()),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_pseudo_element_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::PseudoElementSelector { .. })
    }

    #[must_use]
    pub fn as_pseudo_element_selector(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::PseudoElementSelector { value, .. } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_combinator(&self) -> bool {
        matches!(&self.node_type, NodeType::Combinator { .. })
    }

    #[must_use]
    pub fn as_combinator(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::Combinator { value } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_dimension(&self) -> bool {
        matches!(&self.node_type, NodeType::Dimension { .. })
    }

    #[must_use]
    pub fn as_dimension(&self) -> Option<(&Number, &CompactString)> {
        match &self.node_type {
            NodeType::Dimension { value, unit } => Some((value, unit)),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_id_selector(&self) -> bool {
        matches!(&self.node_type, NodeType::IdSelector { .. })
    }

    #[must_use]
    pub fn as_id_selector(&self) -> Option<&CompactString> {
        match &self.node_type {
            NodeType::IdSelector { value } => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_declaration(&self) -> bool {
        matches!(&self.node_type, NodeType::Declaration { .. })
    }

    #[must_use]
    pub fn as_declaration(&self) -> Option<(&CompactString, &Vec<Node>, &bool)> {
        match &self.node_type {
            NodeType::Declaration {
                property,
                value,
                important,
            } => Some((property, value, important)),
            _ => None,
        }
    }
}

impl Display for Node {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match &self.node_type {
            NodeType::SelectorList { selectors } => selectors
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<String>>()
                .join(", "),
            NodeType::Selector { children } => children
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<String>(),
            NodeType::IdSelector { value } => value.to_string(),
            NodeType::Ident { value } => value.to_string(),
            NodeType::Number { value, .. } => value.to_string(),
            NodeType::Percentage { value } => format!("{value}%"),
            NodeType::Dimension { value, unit } => format!("{value}{unit}"),
            NodeType::Hash { value } => format!("#{}", value.clone()),
            NodeType::String { value } => value.to_string(),
            NodeType::Url { url } => url.to_string(),
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
            // A functional pseudo-element carries its selector list, and dropping it here made
            // `::slotted(.item)` serialize as the bare `::slotted` - a different selector.
            NodeType::PseudoElementSelector { value, arguments } => match arguments {
                Some(arguments) => format!("::{value}({arguments})"),
                None => format!("::{value}"),
            },
            NodeType::Operator { value, .. } => value.to_string(),
            NodeType::ClassSelector { value } => format!(".{value}"),
            NodeType::TypeSelector { namespace, value } => {
                let ns = namespace.as_ref().map_or(String::new(), |ns| format!("{ns}|"));
                format!("{ns}{value}")
            }
            NodeType::Combinator { value } => value.to_string(),
            NodeType::Nth { nth, selector } => {
                let sel = selector
                    .as_ref()
                    .map_or(String::new(), std::string::ToString::to_string);
                format!("{nth}{sel}")
            }
            NodeType::AnPlusB { a, b } => format!("{a}n+{b}"),
            NodeType::Calc { tokens } => {
                let body: Vec<String> = tokens.iter().map(std::string::ToString::to_string).collect();
                format!("calc({})", body.join(" "))
            }
            NodeType::Raw { value } => value.to_string(),

            _ => {
                String::new()
                // panic!("cannot convert to string: {:?}", self)
            }
        };

        write!(f, "{s}")
    }
}
