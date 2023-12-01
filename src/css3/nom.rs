use nom::bytes::complete::{take, take_while, take_while1};
use nom::IResult;
use crate::css3::span::Span;
use crate::css3::tokenizer::Token;

pub mod selector;
pub mod media_query;

pub fn any(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

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
pub fn any_function(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Function(name) = span.to_token() {
        return Ok((input, name));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any string
pub fn any_string(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::QuotedString(qs) = span.to_token() {
        return Ok((input, qs));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns a comma
pub fn comma(input: Span) -> IResult<Span, Span> {
    let (input, span) = take(1usize)(input)?;
    let (input, _) = whitespace0(input)?;

    if let Token::Comma = span.to_token() {
        return Ok((input, span));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::IsNot,
    )))
}

/// Returns any hash
pub fn any_hash(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Hash(h) = span.to_token() {
        return Ok((input, h));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns any delimiter
pub fn any_delim(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Delim(c) = span.to_token() {
        return Ok((input, format!("{}", c)));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::Tag,
    )))
}

/// Returns one or more whitespaces
pub fn whitespace1(input: Span) -> IResult<Span, Span> {
    take_while1(|t: &Token| t == &Token::Whitespace)(input)
}

/// Returns 0 or more whitespaces
pub fn whitespace0(input: Span) -> IResult<Span, Span> {
    take_while(|t: &Token| t == &Token::Whitespace)(input)
}

/// Returns the given delimiter
pub fn delim(input: Span, delim: char) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Delim(c) = span.to_token() {
        if c == delim {
            return Ok((input, format!("{}", c)));
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::IsNot)))
}

/// Returns any identifier
pub fn any_ident(input: Span) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Ident(s) = span.to_token() {
        return Ok((input, s));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::IsNot,
    )))
}

/// Returns the identifier if it matches the given string.
pub fn ident(input: Span, ident: String) -> IResult<Span, String> {
    let (input, span) = take(1usize)(input)?;

    if let Token::Ident(s) = span.to_token() {
        if s == ident {
            return Ok((input, s));
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input.clone(),
        nom::error::ErrorKind::IsNot,
    )))
}