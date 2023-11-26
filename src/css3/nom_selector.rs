use crate::css3::ast::{Node, Span};
use crate::css3::new_tokenizer::Token;
use nom::branch::alt;
use nom::combinator::{map, opt};
use nom::multi::{many0, many1};
use nom::IResult;
use nom::InputTake;

/*

[X] <selector-list> = <complex-selector-list>
[X] <complex-selector-list> = <complex-selector>#
[X] <compound-selector-list> = <compound-selector>#
[X] <simple-selector-list> = <simple-selector>#
[X] <relative-selector-list> = <relative-selector>#
[ ] <complex-selector> = <compound-selector> [ <combinator>? <compound-selector> ]*
[ ] <relative-selector> = <combinator>? <complex-selector>
[ ] <compound-selector> = [ <type-selector>? <subclass-selector>* [ <pseudo-element-selector> <pseudo-class-selector>* ]* ]!
[ ] <simple-selector> = <type-selector> | <subclass-selector>
[-] <combinator> = '>' | '+' | '~' | [ '|' '|' ]
[ ] <type-selector> = <wq-name> | <ns-prefix>? '*'
[X] <ns-prefix> = [ <ident-token> | '*' ]? '|'
[X] <wq-name> = <ns-prefix>? <ident-token>
[ ] <subclass-selector> = <id-selector> | <class-selector> | <attribute-selector> | <pseudo-class-selector>
[ ] <id-selector> = <hash-token>
[ ] <class-selector> = '.' <ident-token>
[ ] <attribute-selector> = '[' <wq-name> ']' | '[' <wq-name> <attr-matcher> [ <string-token> | <ident-token> ] <attr-modifier>? ']'
[ ] <attr-matcher> = [ '~' | '|' | '^' | '$' | '*' ]? '='
[ ] <attr-modifier> = i | s
[ ] <pseudo-class-selector> = ':' <ident-token> | ':' <function-token> <any-value> ')'
[ ] <pseudo-element-selector> = ':' <pseudo-class-selector>

 */

fn parse_selector_list(input: Span) -> IResult<Span, Node> {
    parse_complex_selector_list(input)
}

fn parse_complex_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_complex_selector)(input)?;

    let mut node = Node::new("SelectorList");
    node.children = selectors;

    Ok((input, node))
}

fn parse_relative_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_relative_selector)(input)?;

    let mut node = Node::new("RelativeSelectorList");
    node.children = selectors;

    Ok((input, node))
}

fn parse_relative_selector(input: Span) -> IResult<Span, Node> {
    let node = Node::new("RelativeSelector");
    Ok((input, node))
}

fn parse_complex_selector(input: Span) -> IResult<Span, Node> {
    let (input, first_selector) = parse_compound_selector(input)?;

    let (input, other_selectors) = many0(|i| {
        let (i, combinator) = opt(|i| parse_combinator(i))(i)?;
        let (i, selector) = parse_compound_selector(i)?;

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

    let mut selectors = vec![first_selector];
    for selector in other_selectors {
        selectors.push(selector);
    }

    let mut node = Node::new("ComplexSelector");
    node.children = selectors;

    Ok((input, node))
}

fn parse_compound_selector(input: Span) -> IResult<Span, Node> {
    let (i, selectors) = many1(|i| {
        let (i, _type_selector) = opt(|i| parse_type_selector(i))(i)?;
        let (i, _subclass_selectors) = many0(|i| parse_subclass_selector(i))(i)?;
        let (i, _pseudo_element_and_class_selectors) = many0(|i| {
            let (i, pseudo_element_selector) = parse_pseudo_element_selector(i)?;
            let (i, pseudo_class_selector) = many0(|i| parse_pseudo_class_selector(i))(i)?;

            Ok((i, (pseudo_element_selector, pseudo_class_selector)))
        })(i)?;

        let compound = Node::new("compoundSelector");
        Ok((i, compound))
    })(input)?;

    let mut node = Node::new("CompoundSelector");
    node.children = selectors;

    Ok((i, node))
}

fn wq_name(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i| ns_prefix(i))(input)?;
    let (input, name) = any_ident(input)?;

    let ns_prefix = ns_prefix.unwrap_or("".into());
    let name = name.to_string();

    Ok((input, format!("{}{}", ns_prefix, name)))
}

fn parse_type_selector(input: Span) -> IResult<Span, Node> {
    let (i, type_selector) = alt((wq_name, ns_prefix_star))(input)?;

    let mut node = Node::new("TypeSelector");
    node.attributes.insert("name".into(), type_selector);

    Ok((i, node))
}

fn ns_prefix_star(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i| ns_prefix(i))(input)?;
    let (input, _) = delim(input, '*')?;

    Ok((input, format!("{}*", ns_prefix.unwrap_or("".into()))))
}

fn ns_prefix(input: Span) -> IResult<Span, String> {
    let (input, ns_prefix) = opt(|i| alt((|i| any_ident(i), |i| delim(i, '*')))(i))(input)?;

    let (input, _) = delim(input, '|')?;

    Ok((input, format!("{}|", ns_prefix.unwrap_or("".into()))))
}

fn parse_subclass_selector(input: Span) -> IResult<Span, Node> {
    alt((
        parse_class_selector,
        parse_id_selector,
        parse_attribute_selector,
        parse_pseudo_class_selector,
        parse_pseudo_element_selector,
    ))(input)
}

// fn parse_identifier(input: Span) -> IResult<Span, Node> {
//     let (input, ident) = ident(input, Some("div".into()))?;
//
//     let mut node = Node::new("Identifier");
//     node.attributes.insert("name".into(), ident.clone());
//
//     Ok((input, node))
// }

fn parse_class_selector(input: Span) -> IResult<Span, Node> {
    let (input, _) = delim(input, '.')?; // TODO: check if this is correct
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("ClassSelector");
    node.attributes.insert("name".into(), format!(".{}", name));

    Ok((input, node))
}

fn any_delim(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    if let Token::Delim(c) = span.to_token() {
        return Ok((input, format!("{}", c)));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/*
fn ws<'a, F: 'a, O, E: ParseError<Span<'a>>>(inner: F) -> impl FnMut(Span) -> IResult<Span, O, E>
    where
        F: Fn(Span) -> IResult<Span, O, E>,
{
    delimited(
        multispace0,
        inner,
        multispace0
    )
}
*/
/*
fn multispace0(input: Span) -> IResult<Span, bool> {
    while !input.is_empty() {
        let (t, input) = input.take_split(1);
        if let Token::Whitespace = t.to_token() {
            continue;
        }

        break;
    }

    Ok((input, true))
}
*/

fn delim(input: Span, delim: char) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    let t = span.to_token();
    println!("{:?}", t);

    if let Token::Delim(c) = span.to_token() {
        if c == delim {
            return Ok((input, format!("{}", c)));
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any identifier
fn any_ident(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    let t = span.to_token();
    println!("{:?}", t);

    if let Token::Ident(s) = span.to_token() {
        return Ok((input, s));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns the identifier if it matches the given string.
fn ident(input: Span, ident: String) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    let t = span.to_token();
    println!("{:?}", t);

    if let Token::Ident(s) = span.to_token() {
        if s == ident {
            return Ok((input, s));
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

fn parse_id_selector(input: Span) -> IResult<Span, Node> {
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("IdSelector");
    node.attributes.insert("name".into(), format!("#{}", name));

    Ok((input, node))
}

fn parse_attribute_selector(input: Span) -> IResult<Span, Node> {
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("AttributeSelector");
    node.attributes.insert("name".into(), format!("[{}]", name));

    Ok((input, node))
}

fn parse_pseudo_class_selector(input: Span) -> IResult<Span, Node> {
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("PseudoClassSelector");
    node.attributes.insert("name".into(), format!(":{}", name));

    Ok((input, node))
}

fn parse_pseudo_element_selector(input: Span) -> IResult<Span, Node> {
    let (input, name) = any_ident(input)?;

    let mut node = Node::new("PseudoElementSelector");
    node.attributes.insert("name".into(), format!("::{}", name));

    Ok((input, node))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::{CharIterator, Encoding};
    use crate::css3::new_tokenizer::Tokenizer;

    #[test]
    fn test_parse_selector_list() {
        let mut it = CharIterator::new();
        // it.read_from_str("div > span + span ~ span || span", Some(Encoding::UTF8));
        it.read_from_str(
            "
            hr .short, hr .long {
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
