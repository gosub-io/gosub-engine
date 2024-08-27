use crate::node::{Node, NodeType};
use std::io::Write;
use std::ops::Deref;

/// The walker is used to walk the AST and print it to stdout.
#[allow(dead_code)]
pub struct Walker<'a> {
    root: &'a Node,
}

impl<'a> Walker<'a> {
    #[allow(dead_code)]
    pub fn new(root: &'a Node) -> Self {
        Self { root }
    }

    #[allow(dead_code)]
    pub fn walk_stdout(&self) {
        let _ = inner_walk(self.root, 0, &mut std::io::stdout());
    }

    #[allow(dead_code)]
    pub fn walk_to_string(&self) -> String {
        let mut output: Vec<u8> = Vec::new();

        let _ = inner_walk(self.root, 0, &mut output);

        output.into_iter().map(|c| c as char).collect()
    }
}

fn inner_walk(node: &Node, depth: usize, f: &mut dyn Write) -> Result<(), std::io::Error> {
    let prefix = " ".repeat(depth * 2);

    match node.node_type.deref() {
        NodeType::StyleSheet { children } => {
            writeln!(f, "{}[Stylesheet ({})]", prefix, children.len())?;
            for child in children.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Rule { prelude, block } => {
            writeln!(f, "{}[Rule]", prefix)?;
            // writeln!(f, "{}  - prelude: ", prefix)?;
            inner_walk(prelude.as_ref().unwrap(), depth + 1, f)?;
            // writeln!(f, "{}  - block: ", prefix)?;
            inner_walk(block.as_ref().unwrap(), depth + 1, f)?;
        }
        NodeType::AtRule {
            name,
            prelude,
            block,
        } => {
            writeln!(f, "{}[AtRule] name: {}", prefix, name)?;
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
                "{}[Declaration] property: {} important: {}",
                prefix, property, important
            )?;
            for child in value.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Block { children } => {
            writeln!(f, "{}[Block]", prefix)?;
            for child in children.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Comment { .. } => {}
        // NodeType::Cdo => {}
        // NodeType::Cdc => {}
        NodeType::IdSelector { .. } => {}
        NodeType::Ident { value } => {
            writeln!(f, "{}[Ident] {}", prefix, value)?;
        }
        NodeType::Number { value } => {
            writeln!(f, "{}[Number] {}", prefix, value)?;
        }
        NodeType::Percentage { value } => {
            writeln!(f, "{}[Percentage] {}", prefix, value)?;
        }
        NodeType::Dimension { value, unit } => {
            writeln!(f, "{}[Dimension] {}{}", prefix, value, unit)?;
        }
        NodeType::Prelude => {}
        NodeType::SelectorList { selectors } => {
            writeln!(f, "{}[SelectorList ({})]", prefix, selectors.len())?;
            for child in selectors.iter() {
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
                "{}[AttributeSelector] name: {} value: {} flags: {}",
                prefix, name, value, flags
            )?;
            if matcher.is_some() {
                inner_walk(matcher.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::ClassSelector { value } => {
            writeln!(f, "{}[ClassSelector] {}", prefix, value)?;
        }
        NodeType::NestingSelector => {
            writeln!(f, "{}[NestingSelector]", prefix)?;
        }
        NodeType::TypeSelector { namespace, value } => {
            writeln!(
                f,
                "{}[TypeSelector] namespace: {:?} value: {}",
                prefix, namespace, value
            )?;
        }
        NodeType::Combinator { value } => {
            writeln!(f, "{}[Combinator] {}", prefix, value)?;
        }
        NodeType::Selector { children } => {
            writeln!(f, "{}[Selector]", prefix)?;
            for child in children.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::PseudoElementSelector { value } => {
            writeln!(f, "{}[PseudoElementSelector] {}", prefix, value)?;
        }
        NodeType::PseudoClassSelector { value } => {
            writeln!(f, "{}[PseudoClassSelector]", prefix)?;
            inner_walk(value, depth + 1, f)?;
        }
        NodeType::MediaQuery {
            modifier,
            media_type,
            condition,
        } => {
            writeln!(
                f,
                "{}[MediaQuery] modifier: {} media_type: {}",
                prefix, modifier, media_type
            )?;
            if condition.is_some() {
                inner_walk(condition.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::MediaQueryList { media_queries } => {
            writeln!(f, "{}[MediaQueryList ({})]", prefix, media_queries.len())?;
            for child in media_queries.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Condition { list } => {
            writeln!(f, "{}[Condition ({})]", prefix, list.len())?;
            for child in list.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Feature { kind, name, value } => {
            writeln!(f, "{}[Feature] kind: {:?} name: {}", prefix, kind, name)?;
            if value.is_some() {
                inner_walk(value.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::Hash { value } => {
            writeln!(f, "{}[Hash] {}", prefix, value)?;
        }
        NodeType::Value { children } => {
            writeln!(f, "{}[Value]", prefix)?;
            for child in children.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Comma => {
            writeln!(f, "{}[Comma]", prefix)?;
        }
        NodeType::String { value } => {
            writeln!(f, "{}[String] {}", prefix, value)?;
        }
        NodeType::Url { url } => {
            writeln!(f, "{}[Url] {}", prefix, url)?;
        }
        NodeType::Function { name, arguments } => {
            writeln!(f, "{}[Function] {}", prefix, name)?;
            for child in arguments.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Operator(value) => {
            writeln!(f, "{}[Operator] {}", prefix, value)?;
        }
        NodeType::Nth { nth, selector } => {
            writeln!(f, "{}[Nth]", prefix)?;
            inner_walk(nth, depth + 1, f)?;
            if selector.is_some() {
                inner_walk(selector.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::AnPlusB { a, b } => {
            writeln!(f, "{}[AnPlusB] a: {} b: {}", prefix, a, b)?;
        }
        NodeType::MSFunction { func } => {
            writeln!(f, "{}[MSFunction]", prefix)?;
            inner_walk(func, depth + 1, f)?;
        }
        NodeType::MSIdent {
            value,
            default_value,
        } => {
            writeln!(
                f,
                "{}[MSIdent] value: {} default_value: {}",
                prefix, value, default_value
            )?;
        }
        NodeType::Calc { expr } => {
            writeln!(f, "{}[Calc]", prefix)?;
            inner_walk(expr, depth + 1, f)?;
        }
        NodeType::SupportsDeclaration { term } => {
            writeln!(f, "{}[SupportsDeclaration]", prefix)?;
            inner_walk(term, depth + 1, f)?;
        }
        NodeType::FeatureFunction => {
            writeln!(f, "{}[FeatureFunction]", prefix)?;
        }

        NodeType::Raw { value } => {
            writeln!(f, "{}[Raw] {}", prefix, value)?;
        }
        NodeType::Scope { root, limit } => {
            writeln!(f, "{}[Scope]", prefix)?;
            if root.is_some() {
                inner_walk(root.as_ref().unwrap(), depth + 1, f)?;
            }
            if limit.is_some() {
                inner_walk(limit.as_ref().unwrap(), depth + 1, f)?;
            }
        }
        NodeType::LayerList { layers } => {
            writeln!(f, "{}[LayerList]", prefix)?;
            for child in layers.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::ImportList { children } => {
            writeln!(f, "{}[ImportList]", prefix)?;
            for child in children.iter() {
                inner_walk(child, depth + 1, f)?;
            }
        }
        NodeType::Container { children } => {
            writeln!(f, "{}[Container]", prefix)?;
            for child in children.iter() {
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
            writeln!(f, "{}[Range]", prefix)?;
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
