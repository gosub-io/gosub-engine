use crate::css3::ast::Node;
use nom::combinator::{opt};
use nom::IResult;
use crate::css3::nom::{number};
use crate::css3::span::Span;

/// This module contains functions to parse values
/// https://www.w3.org/TR/css-values-4


pub fn parse_ratio(input: Span) -> IResult<Span, Node> {
    let (input, a) = number(input)?;
    let (input, b) = opt(|i| number(i))(input)?;

    let mut node = Node::new("ratio");
    node.children.push(a);
    if let Some(b) = b {
        node.children.push(b);
    } else {
        // Always add a default value
        let mut default_nr = Node::new("number");
        default_nr.attributes.insert("value".to_string(), "1".to_string());

        node.children.push(default_nr);
    }

    Ok((input, node))
}