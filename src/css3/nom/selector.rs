use crate::css3::ast::Node;
use crate::css3::tokenizer::Token;
use nom::branch::alt;
use nom::combinator::{map, opt};
use nom::multi::{many0, separated_list1};
use nom::IResult;
use crate::css3::nom::{any, any_function, any_hash, any_ident, any_string, comma, delim, ident, whitespace0, whitespace1};
use crate::css3::span::Span;

/// This module contains functions to parse CSS level 4 selectors. For more information, see:
/// https://www.w3.org/TR/selectors-4

/// <selector-list> = <complex-selector-list>
pub(crate) fn parse_selector_list(input: Span) -> IResult<Span, Node> {
    parse_complex_selector_list(input)
}

/// <complex-selector-list> = <complex-selector>#
fn parse_complex_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = separated_list1(comma, parse_complex_selector)(input)?;

    let mut node = Node::new("ComplexSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <compound-selector-list> = <compound-selector>#
fn parse_compound_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = separated_list1(comma, parse_compound_selector)(input)?;

    let mut node = Node::new("CompoundSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <simple-selector-list> = <simple-selector>#
fn parse_simple_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = separated_list1(comma, parse_simple_selector)(input)?;

    let mut node = Node::new("SimpleSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <relative-selector-list> = <relative-selector>#
fn parse_relative_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = separated_list1(comma, parse_relative_selector)(input)?;

    let mut node = Node::new("RelativeSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <complex-selector> = <compound-selector> [ <combinator>? <compound-selector> ]*
fn parse_complex_selector(input: Span) -> IResult<Span, Node> {
    let (input, first_selector) = parse_compound_selector(input)?;

    let (input, other_selectors) = many0(|i| {
        let (i, combinator) = opt(|i|
            alt((
                |i| parse_combinator(i),
                map(|i| whitespace1(i), |_| Node::new("whitespace")),
            ))(i)
        )(i)?;

        let (i, selector) = parse_compound_selector(i)?;
        let (i, _) = whitespace0(i)?;

        let node = match combinator {
            Some(combinator) => {
                let mut node = Node::new("Combinator");
                node.children.push(combinator);
                node.children.push(selector);
                node
            }
            None => selector,
        };
        Ok((i, node))
    })(input)?;

    let (input, _) = whitespace0(input)?;

    let mut selectors = vec![first_selector];
    for selector in other_selectors {
        selectors.push(selector);
    }

    let mut node = Node::new("ComplexSelector");
    node.children = selectors;

    Ok((input, node))
}

/// <relative-selector> = <combinator>? <complex-selector>
fn parse_relative_selector(input: Span) -> IResult<Span, Node> {
    let (input, combinator) = opt(|i| parse_combinator(i))(input)?;
    let (input, complex_selector) = parse_complex_selector(input)?;
    let (input, _) = whitespace0(input)?;

    let mut node = Node::new("RelativeSelector");
    if combinator.is_some() {
        node.children.push(combinator.unwrap());
    }
    node.children.push(complex_selector);

    Ok((input, node))
}

/// <compound-selector> = [ <type-selector>? <subclass-selector>* [ <pseudo-element-selector> <pseudo-class-selector>* ]* ]!
fn parse_compound_selector(input: Span) -> IResult<Span, Node> {
    let (i, type_selector) = opt(|i| parse_type_selector(i))(input)?;
    let (i, subclass_selectors) = many0(|i| parse_subclass_selector(i))(i)?;
    let (i, pseudo_element_and_class_selectors) = many0(|i| {
        let (i, pseudo_element_selector) = parse_pseudo_element_selector(i)?;
        let (i, pseudo_class_selector) = many0(|i| parse_pseudo_class_selector(i))(i)?;
        Ok((i, (pseudo_element_selector, pseudo_class_selector)))
    })(i)?;
    let (i, _) = whitespace0(i)?;

    if type_selector.is_none() && subclass_selectors.is_empty() && pseudo_element_and_class_selectors.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            i,
            nom::error::ErrorKind::IsNot,
        )));
    }

    let mut node = Node::new("CompoundSelector");

    if type_selector.is_some() {
        node.children.push(type_selector.unwrap());
    }
    if !subclass_selectors.is_empty() {
        let mut subclass_selectors_node = Node::new("SubclassSelectors");
        subclass_selectors_node.children = subclass_selectors;
        node.children.push(subclass_selectors_node);
    }
    if !pseudo_element_and_class_selectors.is_empty() {
        let mut pseudo_element_and_class_selectors_node = Node::new("PseudoElementAndClassSelectors");
        for (pseudo_element_selector, pseudo_class_selectors) in pseudo_element_and_class_selectors {
            let mut pseudo_element_and_class_selector_node = Node::new("PseudoElementAndClassSelector");
            pseudo_element_and_class_selector_node.children.push(pseudo_element_selector);
            if !pseudo_class_selectors.is_empty() {
                let mut pseudo_class_selectors_node = Node::new("PseudoClassSelectors");
                pseudo_class_selectors_node.children = pseudo_class_selectors;
                pseudo_element_and_class_selector_node.children.push(pseudo_class_selectors_node);
            }
            pseudo_element_and_class_selectors_node.children.push(pseudo_element_and_class_selector_node);
        }
        node.children.push(pseudo_element_and_class_selectors_node);
    }

    Ok((i, node))
}

/// <simple-selector> = <type-selector> | <subclass-selector>
fn parse_simple_selector(input: Span) -> IResult<Span, Node> {
    alt((
        parse_type_selector,
        parse_subclass_selector,
    ))(input)
}

/// <combinator> = '>' | '+' | '~' | [ '|' '|' ]
fn parse_combinator(input: Span) -> IResult<Span, Node> {
    alt((
        map(|i| delim(i, '<'), |_| Node::new("ChildCombinator")),
        map(
            |i| delim(i, '+'),
            |_| Node::new("AdjacentSiblingCombinator"),
        ),
        map(|i| delim(i, '~'), |_| Node::new("GeneralSiblingCombinator")),
        map(|i| ident(i, "||".into()), |_| Node::new("ColumnCombinator")),
    ))(input)
}

/// <type-selector> = <wq-name> | <ns-prefix>? '*'
fn parse_type_selector(input: Span) -> IResult<Span, Node> {
    let (i, type_selector) = alt((
        wq_name,
        ns_prefix_star
    ))(input)?;

    let mut node = Node::new("TypeSelector");
    node.attributes.insert("name".into(), type_selector);

    Ok((i, node))
}

/// <ns-prefix>? '*'
fn ns_prefix_star(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i| ns_prefix(i))(input)?;
    let (input, _) = delim(input, '*')?;

    Ok((input, format!("{}*", ns_prefix.unwrap_or("".into()))))
}

/// <ns-prefix> = [ <ident-token> | '*' ]? '|'
fn ns_prefix(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i|
        alt((
            |i| any_ident(i),
            |i| delim(i, '*')
        ))(i)
    )(input)?;

    let (input, _) = delim(input, '|')?;

    Ok((input, format!("{}|", ns_prefix.unwrap_or("".into()))))
}

/// <wq-name> = <ns-prefix>? <ident-token>
fn wq_name(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i| ns_prefix(i))(input)?;
    let (input, name) = any_ident(input)?;

    let ns_prefix = ns_prefix.unwrap_or("".into());
    let name = name.to_string();

    Ok((input, format!("{}{}", ns_prefix, name)))
}

/// <subclass-selector> = <id-selector> | <class-selector> | <attribute-selector> | <pseudo-class-selector>
fn parse_subclass_selector(input: Span) -> IResult<Span, Node> {
    alt((
        parse_id_selector,
        parse_class_selector,
        parse_attribute_selector,
        parse_pseudo_class_selector,
    ))(input)
}

/// <id-selector> = <hash-token>
fn parse_id_selector(input: Span) -> IResult<Span, Node> {
    let (input, name) = any_hash(input)?;

    let mut node = Node::new("IdSelector");
    node.attributes.insert("name".into(), format!("#{}", name));

    Ok((input, node))
}

/// <class-selector> = '.' <ident-token>
fn parse_class_selector(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, '.')?;
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("ClassSelector");
    node.attributes.insert("name".into(), format!(".{}", name));

    Ok((input, node))
}

/// <attribute-selector> = '[' <wq-name> ']' | '[' <wq-name> <attr-matcher> [ <string-token> | <ident-token> ] <attr-modifier>? ']'
fn parse_attribute_selector(input: Span) -> IResult<Span, Node> {
    alt((
        parse_attribute_selector_no_value,
        parse_attribute_selector_with_value,
    ))(input)
}

/// '[' <wq-name> ']'
fn parse_attribute_selector_no_value(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, '[')?;
    let (input, name) = wq_name(input)?;
    let (input, _) = delim(input, ']')?;

    let mut node = Node::new("AttributeSelector");
    node.attributes.insert("name".into(), format!("[{}]", name));
    Ok((input, node))
}

/// '[' <wq-name> <attr-matcher> [ <string-token> | <ident-token> ] <attr-modifier>? ']'
fn parse_attribute_selector_with_value(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, '[')?;
    let (input, name) = wq_name(input)?;
    let (input, matcher) = parse_attr_matcher(input)?;
    let (input, value) = alt((
        |i| any_string(i),
        |i| any_ident(i),
    ))(input)?;
    let (input, modifier) = opt(|i| parse_attr_modifier(i))(input)?;
    let (input, _) = delim(input, ']')?;


    let mut node = Node::new("AttributeSelector");
    node.attributes.insert("name".into(), format!("[{}]", name));
    node.attributes.insert("value".into(), format!("{}", value));
    if matcher.is_some() {
        node.attributes.insert("matcher".into(), format!("{}", matcher.unwrap()));
    }
    if modifier.is_some() {
        node.attributes.insert("modifier".into(), format!("{}", modifier.unwrap()));
    }

    Ok((input, node))
}

/// <attr-matcher> = [ '~' | '|' | '^' | '$' | '*' ]? '='
fn parse_attr_matcher(input: Span) -> IResult<Span, Option<String>> {
    let (input, matcher) = opt(|i|
        alt((
            |i| delim(i, '~'),
            |i| delim(i, '|'),
            |i| delim(i, '^'),
            |i| delim(i, '$'),
            |i| delim(i, '*'),
        ))(i)
    )(input)?;

    let (input, _ ) = delim(input, '=')?;

    return Ok((input, matcher));
}

/// <attr-modifier> = i | s
fn parse_attr_modifier(input: Span) -> IResult<Span, String> {
    let (input, modifier) = alt((
        |i| ident(i, "i".into()),
        |i| ident(i, "s".into()),
    ))(input)?;

    Ok((input, modifier))
}

/// <pseudo-class-selector> = ':' <ident-token> | ':' <function-token> <any-value> ')'
fn parse_pseudo_class_selector(input: Span) -> IResult<Span, Node> {
    alt((
        parse_pseudo_class_selector_ident,
        parse_pseudo_class_selector_function,
    ))(input)
}

/// <pseudo-class-selector> = ':' <ident-token>
fn parse_pseudo_class_selector_ident(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, ':')?;
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("PseudoClassSelector");
    node.attributes.insert("name".into(), format!(":{}", name));

    Ok((input, node))
}

/// ':' <function-token> <any-value> ')'
fn parse_pseudo_class_selector_function(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, ':')?;
    let (input, name) = any_function(input)?;
    let (input, value) = any(input)?;
    let (input, _) = delim(input, ')')?;

    let mut node = Node::new("PseudoClassSelector");
    node.attributes.insert("name".into(), format!(":{}({})", name, value));

    Ok((input, node))
}

/// <pseudo-element-selector> = ':' <pseudo-class-selector>
fn parse_pseudo_element_selector(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, ':')?;
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("PseudoElementSelector");
    node.attributes.insert("name".into(), format!("::{}", name));

    Ok((input, node))
}

// =================================================================================================
// These functions will return direct strings from the wanted tokens

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{CharIterator, Encoding};
    use crate::css3::tokenizer::Tokenizer;

    #[test]
    fn test_parse_selector_list() {
        let mut it = CharIterator::new();
        // it.read_from_str("div > span + span ~ span || span { color: red } ", Some(Encoding::UTF8));
        it.read_from_str("\
* {
    box-sizing: border-box
}

.main-content h2 {
    color: #3c4040;
    font-weight: 400;
    line-height: 1.5rem;
    font-size: 1rem;
    padding-right: 60px
}
", Some(Encoding::UTF8));
/*
        it.read_from_str(
            "
            ahr .short, bhr .long {
            background-color: var(--border-base-color);
            border: 0;
            color: var(--border-base-color);
            height: 1px;
            margin: 20px 0 0 0;
            overflow: hidden;
            padding: 0;
            text-align: left;
            width: 65px
        }",
            Some(Encoding::UTF8),
        );
*/
        let mut tokenizer = Tokenizer::new(&mut it);
        tokenizer.consume_all();
        let tokens = tokenizer.tokens;
        let tokens = tokens
            .iter()
            .cloned()
            .filter(|t| *t != Token::Whitespace)
            .collect();

        let input = Span::new(&tokens);
        let (input, node) = parse_selector_list(input).unwrap();

        println!("{:?}", input);
        println!("{:#?}", node);
    }
}
