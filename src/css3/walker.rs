use crate::css3::node::{Node, NodeType};
use std::ops::Deref;

pub struct Walker;

impl Walker {
    pub fn walk(&self, node: &Node) {
        inner_walk(node, 0);
    }
}

fn inner_walk(node: &Node, depth: usize) {
    let prefix = " ".repeat(depth * 2);

    match node.node_type.deref() {
        NodeType::StyleSheet { children } => {
            println!("{}[Stylesheet ({})]", prefix, children.len());
            for child in children.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Rule { prelude, block } => {
            println!("{}[Rule]", prefix);
            // println!("{}  - prelude: ", prefix);
            inner_walk(prelude.as_ref().unwrap(), depth + 1);
            // println!("{}  - block: ", prefix);
            inner_walk(block.as_ref().unwrap(), depth + 1);
        }
        NodeType::AtRule { name, prelude, block } => {
            println!("{}[AtRule] name: {}", prefix, name);
            if prelude.is_some() {
                inner_walk(prelude.as_ref().unwrap(), depth + 1);
            }
            if block.is_some() {
                inner_walk(block.as_ref().unwrap(), depth + 1);
            }
        }
        NodeType::Declaration {
            property,
            value,
            important,
        } => {
            println!(
                "{}[Declaration] property: {} important: {}",
                prefix, property, important
            );
            for child in value.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Block { children } => {
            println!("{}[Block]", prefix);
            for child in children.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Comment { .. } => {}
        NodeType::Cdo => {}
        NodeType::Cdc => {}
        NodeType::IdSelector { .. } => {}
        NodeType::Ident { value } => {
            println!("{}[Ident] {}", prefix, value);
        }
        NodeType::Number { value } => {
            println!("{}[Number] {}", prefix, value);
        }
        NodeType::Percentage { value } => {
            println!("{}[Percentage] {}", prefix, value);
        }
        NodeType::Dimension { value, unit } => {
            println!("{}[Dimension] {}{}", prefix, value, unit);
        }
        NodeType::Prelude => {}
        NodeType::SelectorList { selectors } => {
            println!("{}[SelectorList ({})]", prefix, selectors.len());
            for child in selectors.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::AttributeSelector {
            name,
            matcher,
            value,
            flags,
        } => {
            println!(
                "{}[AttributeSelector] name: {} value: {} flags: {}",
                prefix, name, value, flags
            );
            if matcher.is_some() {
                inner_walk(matcher.as_ref().unwrap(), depth + 1);
            }
        }
        NodeType::ClassSelector { value } => {
            println!("{}[ClassSelector] {}", prefix, value);
        }
        NodeType::NestingSelector => {
            println!("{}[NestingSelector]", prefix);
        }
        NodeType::TypeSelector { namespace, value } => {
            println!(
                "{}[TypeSelector] namespace: {:?} value: {}",
                prefix, namespace, value
            );
        }
        NodeType::Combinator { value } => {
            println!("{}[Combinator] {}", prefix, value);
        }
        NodeType::Selector { children } => {
            println!("{}[Selector]", prefix);
            for child in children.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::PseudoElementSelector { value } => {
            println!("{}[PseudoElementSelector] {}", prefix, value);
        }
        NodeType::PseudoClassSelector { value } => {
            println!("{}[PseudoClassSelector]", prefix);
            inner_walk(value, depth + 1);
        }
        NodeType::MediaQuery {
            modifier,
            media_type,
            condition,
        } => {
            println!(
                "{}[MediaQuery] modifier: {} media_type: {}",
                prefix, modifier, media_type
            );
            if condition.is_some() {
                inner_walk(condition.as_ref().unwrap(), depth + 1);
            }
        }
        NodeType::MediaQueryList { media_queries } => {
            println!("{}[MediaQueryList ({})]", prefix, media_queries.len());
            for child in media_queries.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Condition { list } => {
            println!("{}[Condition ({})]", prefix, list.len());
            for child in list.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Feature { kind, name, value } => {
            println!("{}[Feature] kind: {:?} name: {}", prefix, kind, name);
            if value.is_some() {
                inner_walk(value.as_ref().unwrap(), depth + 1);
            }
        }
        NodeType::Hash { value } => {
            println!("{}[Hash] {}", prefix, value);
        }
        NodeType::Value { children } => {
            println!("{}[Value]", prefix);
            for child in children.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Comma => {
            println!("{}[Comma]", prefix);
        }
        NodeType::String { value } => {
            println!("{}[String] {}", prefix, value);
        }
        NodeType::Url { url } => {
            println!("{}[Url] {}", prefix, url);
        }
        NodeType::Function { name, arguments } => {
            println!("{}[Function] {}", prefix, name);
            for child in arguments.iter() {
                inner_walk(child, depth + 1);
            }
        }
        NodeType::Operator(value) => {
            println!("{}[Operator] {}", prefix, value);
        }
        NodeType::Nth { nth, selector } => {
            println!("{}[Nth]", prefix);
            inner_walk(nth, depth + 1);
            if selector.is_some() {
                inner_walk(selector.as_ref().unwrap(), depth + 1);
            }
        }
        NodeType::AnPlusB { a, b } => {
            println!("{}[AnPlusB] a: {} b: {}", prefix, a, b);
        }
        NodeType::MSFunction { func } => {
            println!("{}[MSFunction]", prefix);
            inner_walk(func, depth + 1);
        }
        NodeType::MSIdent { value, default_value } => {
            println!(
                "{}[MSIdent] value: {} default_value: {}",
                prefix, value, default_value
            );
        }
        NodeType::Calc { expr } => {
            println!("{}[Calc]", prefix);
            inner_walk(expr, depth + 1);
        }
    }
}
