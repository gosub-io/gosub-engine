use crate::node::{Node, NodeType};
use std::io::Write;

/// The walker is used to walk the AST and print it to stdout.
#[allow(dead_code)]
pub struct Walker<'a> {
    root: &'a Node,
}

impl<'a> Walker<'a> {
    #[allow(dead_code)]
    #[must_use] 
    pub fn new(root: &'a Node) -> Self {
        Self { root }
    }

    #[allow(dead_code)]
    pub fn walk_stdout(&self) {
        let _ = inner_walk(self.root, 0, &mut std::io::stdout());
    }

    #[allow(dead_code)]
    #[must_use] 
    pub fn walk_to_string(&self) -> String {
        let mut output: Vec<u8> = Vec::new();

        let _ = inner_walk(self.root, 0, &mut output);

        output.into_iter().map(|c| c as char).collect()
    }
}

fn inner_walk(node: &Node, depth: usize, f: &mut dyn Write) -> Result<(), std::io::Error> {
    let prefix = " ".repeat(depth * 2);

    match &*node.node_type {
        NodeType::StyleSheet { children } => {
            writeln!(f, "{}[Stylesheet ({})]", prefix, children.len())?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Rule { prelude, block } => {
            writeln!(f, "{prefix}[Rule]")?;
            // writeln!(f, "{}  - prelude: ", prefix)?;
            inner_walk(prelude.as_ref().unwrap(), depth + 1, f)?;
            // writeln!(f, "{}  - block: ", prefix)?;
            inner_walk(block.as_ref().unwrap(), depth + 1, f)?;
        }
        NodeType::AtRule { name, prelude, block } => {
            writeln!(f, "{prefix}[AtRule] name: {name}")?;
            if prelude.is_some() {
                inner_walk(prelude.as_ref().unwrap(), depth + 1, f)?;
            }
            if block.is_some() {
                inner_walk(block.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::Declaration {
            property,
            value,
            important,
        } => {
            writeln!(
                f,
                "{prefix}[Declaration] property: {property} important: {important}"
            )?;
            for child in value {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Block { children } => {
            writeln!(f, "{prefix}[Block]")?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Comment { .. } => {}
        // NodeType::Cdo => {}
        // NodeType::Cdc => {}
        NodeType::IdSelector { .. } => {}
        NodeType::Ident { value } => {
            writeln!(f, "{prefix}[Ident] {value}")?;
        }
        NodeType::Number { value } => {
            writeln!(f, "{prefix}[Number] {value}")?;
        }
        NodeType::Percentage { value } => {
            writeln!(f, "{prefix}[Percentage] {value}")?;
        }
        NodeType::Dimension { value, unit } => {
            writeln!(f, "{prefix}[Dimension] {value}{unit}")?;
        }
        NodeType::Prelude => {}
        NodeType::SelectorList { selectors } => {
            writeln!(f, "{}[SelectorList ({})]", prefix, selectors.len())?;
            for child in selectors {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::AttributeSelector {
            name,
            matcher,
            value,
            flags,
        } => {
            writeln!(
                f,
                "{prefix}[AttributeSelector] name: {name} value: {value} flags: {flags}"
            )?;
            if matcher.is_some() {
                inner_walk(matcher.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::ClassSelector { value } => {
            writeln!(f, "{prefix}[ClassSelector] {value}")?;
        }
        NodeType::NestingSelector => {
            writeln!(f, "{prefix}[NestingSelector]")?;
        }
        NodeType::TypeSelector { namespace, value } => {
            writeln!(
                f,
                "{prefix}[TypeSelector] namespace: {namespace:?} value: {value}"
            )?;
        }
        NodeType::Combinator { value } => {
            writeln!(f, "{prefix}[Combinator] {value}")?;
        }
        NodeType::Selector { children } => {
            writeln!(f, "{prefix}[Selector]")?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::PseudoElementSelector { value } => {
            writeln!(f, "{prefix}[PseudoElementSelector] {value}")?;
        }
        NodeType::PseudoClassSelector { value } => {
            writeln!(f, "{prefix}[PseudoClassSelector]")?;
            inner_walk(value, depth + 1, f)?;
        }
        NodeType::MediaQuery {
            modifier,
            media_type,
            condition,
        } => {
            writeln!(
                f,
                "{prefix}[MediaQuery] modifier: {modifier} media_type: {media_type}"
            )?;
            if condition.is_some() {
                inner_walk(condition.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::MediaQueryList { media_queries } => {
            writeln!(f, "{}[MediaQueryList ({})]", prefix, media_queries.len())?;
            for child in media_queries {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Condition { list } => {
            writeln!(f, "{}[Condition ({})]", prefix, list.len())?;
            for child in list {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Feature { kind, name, value } => {
            writeln!(f, "{prefix}[Feature] kind: {kind:?} name: {name}")?;
            if value.is_some() {
                inner_walk(value.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::Hash { value } => {
            writeln!(f, "{prefix}[Hash] {value}")?;
        }
        NodeType::Value { children } => {
            writeln!(f, "{prefix}[Value]")?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Comma => {
            writeln!(f, "{prefix}[Comma]")?;
        }
        NodeType::String { value } => {
            writeln!(f, "{prefix}[String] {value}")?;
        }
        NodeType::Url { url } => {
            writeln!(f, "{prefix}[Url] {url}")?;
        }
        NodeType::Function { name, arguments } => {
            writeln!(f, "{prefix}[Function] {name}")?;
            for child in arguments {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Operator(value) => {
            writeln!(f, "{prefix}[Operator] {value}")?;
        }
        NodeType::Nth { nth, selector } => {
            writeln!(f, "{prefix}[Nth]")?;
            inner_walk(nth, depth + 1, f)?;
            if selector.is_some() {
                inner_walk(selector.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::AnPlusB { a, b } => {
            writeln!(f, "{prefix}[AnPlusB] a: {a} b: {b}")?;
        }
        NodeType::MSFunction { func } => {
            writeln!(f, "{prefix}[MSFunction]")?;
            inner_walk(func, depth + 1, f)?;
        }
        NodeType::MSIdent { value, default_value } => {
            writeln!(
                f,
                "{prefix}[MSIdent] value: {value} default_value: {default_value}"
            )?;
        }
        NodeType::Calc { expr } => {
            writeln!(f, "{prefix}[Calc]")?;
            inner_walk(expr, depth + 1, f)?;
        }
        NodeType::SupportsDeclaration { term } => {
            writeln!(f, "{prefix}[SupportsDeclaration]")?;
            inner_walk(term, depth + 1, f)?;
        }
        NodeType::FeatureFunction => {
            writeln!(f, "{prefix}[FeatureFunction]")?;
        }

        NodeType::Raw { value } => {
            writeln!(f, "{prefix}[Raw] {value}")?;
        }
        NodeType::Scope { root, limit } => {
            writeln!(f, "{prefix}[Scope]")?;
            if root.is_some() {
                inner_walk(root.as_ref().unwrap(), depth + 1, f)?;
            }
            if limit.is_some() {
                inner_walk(limit.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::LayerList { layers } => {
            writeln!(f, "{prefix}[LayerList]")?;
            for child in layers {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::ImportList { children } => {
            writeln!(f, "{prefix}[ImportList]")?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Container { children } => {
            writeln!(f, "{prefix}[Container]")?;
            for child in children {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Cdo => {}
        NodeType::Cdc => {}
        NodeType::Range {
            left,
            left_comparison,
            middle,
            right_comparison,
            right,
        } => {
            writeln!(f, "{prefix}[Range]")?;
            inner_walk(left, depth + 1, f)?;
            inner_walk(left_comparison, depth + 1, f)?;
            inner_walk(middle, depth + 1, f)?;
            if right_comparison.is_some() {
                inner_walk(right_comparison.as_ref().unwrap(), depth + 1, f)?;
            }
            if right.is_some() {
                inner_walk(right.as_ref().unwrap(), depth + 1, f)?;
            }
        }
    }
    Ok(())
}
