use crate::css3::ast::{Node, Span};
use crate::css3::new_tokenizer::Token;
use nom::branch::alt;
use nom::combinator::{map, opt};
use nom::multi::{many0, many1};
use nom::IResult;
use nom::InputTake;

/// <selector-list> = <complex-selector-list>
fn parse_selector_list(input: Span) -> IResult<Span, Node> {
    parse_complex_selector_list(input)
}

/// <complex-selector-list> = <complex-selector>#
fn parse_complex_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_complex_selector)(input)?;

    let mut node = Node::new("ComplexSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <compound-selector-list> = <compound-selector>#
fn parse_compound_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_compound_selector)(input)?;

    let mut node = Node::new("CompoundSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <simple-selector-list> = <simple-selector>#
fn parse_simple_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_simple_selector)(input)?;

    let mut node = Node::new("SimpleSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <relative-selector-list> = <relative-selector>#
fn parse_relative_selector_list(input: Span) -> IResult<Span, Node> {
    let (input, selectors) = many1(parse_relative_selector)(input)?;

    let mut node = Node::new("RelativeSelectorList");
    node.children = selectors;

    Ok((input, node))
}

/// <complex-selector> = <compound-selector> [ <combinator>? <compound-selector> ]*
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

/// <relative-selector> = <combinator>? <complex-selector>
fn parse_relative_selector(input: Span) -> IResult<Span, Node> {
    let (input, combinator) = opt(|i| parse_combinator(i))(input)?;
    let (input, complex_selector) = parse_complex_selector(input)?;

    let mut node = Node::new("RelativeSelector");
    if combinator.is_some() {
        node.children.push(combinator.unwrap());
    }
    node.children.push(complex_selector);

    Ok((input, node))
}

/// <compound-selector> = [ <type-selector>? <subclass-selector>* [ <pseudo-element-selector> <pseudo-class-selector>* ]* ]!
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
    let (input, ns_prefix) = opt(|i| alt((|i| any_ident(i), |i| delim(i, '*')))(i))(input)?;

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
        ))(input)
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

fn any(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    match span.to_token() {
        Token::Ident(s) => return Ok((input, s)),
        Token::Hash(s) => return Ok((input, s)),
        Token::QuotedString(s) => return Ok((input, s)),
        Token::Delim(c) => return Ok((input, format!("{}", c))),
        Token::Function(s) => return Ok((input, s)),
        Token::AtKeyword(s) => return Ok((input, s)),
        Token::Url(s) => return Ok((input, s)),
        Token::BadUrl(s) => return Ok((input, s)),
        Token::Dimension { value, unit } => return Ok((input, format!("{}{}", value, unit))),
        Token::Percentage(s) => return Ok((input, format!("{}", s))),
        Token::Number(s) => return Ok((input, format!("{}", s))),
        Token::BadString(s) => return Ok((input, s)),
        Token::IDHash(s) => return Ok((input, s)),
        _ => {}
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}


    /// Returns the name of a function token
fn any_function(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    if let Token::Function(name) = span.to_token() {
        return Ok((input, name));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any string
fn any_string(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    if let Token::QuotedString(qs) = span.to_token() {
        return Ok((input, qs));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any hash
fn any_hash(input: Span) -> IResult<Span, String> {
    let (input, span) = input.take_split(1);

    if let Token::Hash(h) = span.to_token() {
        return Ok((input, h));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any delimiter
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

/// Returns the given delimiter
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
