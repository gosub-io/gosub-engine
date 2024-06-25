use crate::errors::Error;
use crate::syntax_matcher::CssSyntaxTree;
use gosub_css3::stylesheet::CssValue;
use gosub_shared::types::Result;
use nom::branch::alt;
use nom::bytes::complete::{tag, tag_no_case, take_while};
use nom::character::complete::{alpha1, alphanumeric1, char, digit0, digit1, multispace0, one_of};
use nom::combinator::{map, map_res, opt, recognize};
use nom::multi::{fold_many1, many1, separated_list0, separated_list1};
use nom::number::complete::float;
use nom::sequence::{delimited, pair, preceded, separated_pair};
use nom::IResult;
use std::fmt::{Display, Formatter};
use nom::Err;

macro_rules! debug_print {
    // ($($x:tt)*) => { println!($($x)*) }
    ($($x:tt)*) => ({})
}


#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Combinator of this group (what should we match from this group?)
    pub combinator: GroupCombinators,
    /// Components in this group
    pub components: Vec<SyntaxComponent>,
}

#[derive(PartialEq, Debug, Clone)]
pub enum GroupCombinators {
    /// All elements must be matched in order (space delimited)
    Juxtaposition,
    /// &&   (all elements must be matched in any order)
    AllAnyOrder,
    /// ||   (at least one element must be matched in any order)
    AtLeastOneAnyOrder,
    /// |    (exactly one element must be matched)
    ExactlyOne,
}

/// Multiplier for a syntax component that defines how many times this component is allowed to appear
#[allow(dead_code)]
#[derive(PartialEq, Debug, Clone)]
pub enum SyntaxComponentMultiplier {
    /// Default case
    Once,
    /// Zero or more: *
    ZeroOrMore,
    /// One or more +
    OneOrMore,
    /// Optional ?
    Optional,
    /// Between (range) {}
    Between(usize, usize),
    /// !
    AtLeastOneValue,
    /// Comma seperated repeat (# with optional {})
    CommaSeparatedRepeat(usize, usize),
}

impl Display for SyntaxComponentMultiplier {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SyntaxComponentMultiplier::Once => write!(f, ""),
            SyntaxComponentMultiplier::ZeroOrMore => write!(f, "*"),
            SyntaxComponentMultiplier::OneOrMore => write!(f, "+"),
            SyntaxComponentMultiplier::Optional => write!(f, "?"),
            SyntaxComponentMultiplier::Between(min, max) => write!(f, "{{{}, {}}}", min, max),
            SyntaxComponentMultiplier::AtLeastOneValue => write!(f, "!"),
            SyntaxComponentMultiplier::CommaSeparatedRepeat(min, max) => write!(f, "#{{{}, {}}}", min, max),
        }
    }
}

/// Defines a syntax component with a multiplier (defaults to Once)
#[derive(PartialEq, Debug, Clone)]
pub struct SyntaxComponent {
    /// Actual component
    pub type_: SyntaxComponentType,
    /// Multiplier(s) for this component (there can be multiple multipliers in some cases)
    pub multipliers: SyntaxComponentMultiplier,
}

impl SyntaxComponent {
    /// Creates a new syntax component
    pub fn new(type_: SyntaxComponentType, multiplier: SyntaxComponentMultiplier) -> SyntaxComponent {
        SyntaxComponent {
            type_: type_,
            multipliers: multiplier,
        }
    }
}

/// Represent either a number (i64) or infinity
#[derive(Debug, PartialEq, Clone)]
enum NumberOrInfinity {
    /// Nothing defined (no min or max)
    None,
    // Finite number (in i64 range)
    FiniteI64(i64),
    // ∞
    Infinity,
    // -∞
    NegativeInfinity,
}

/// Represents an optional min and/or max range for a type definition
#[derive(Clone, Debug, PartialEq)]
pub struct RangeType {
    /// Mininum value
    min: NumberOrInfinity,
    /// Maximum value
    max: NumberOrInfinity,
}

impl RangeType {
    /// Returns an empty range
    fn empty() -> Self {
        RangeType {
            min: NumberOrInfinity::None,
            max: NumberOrInfinity::None,
        }
    }
}

/// Syntax component types. These are the elements that make up the css declaration syntax.
#[derive(PartialEq, Debug, Clone)]
pub enum SyntaxComponentType {
    /// Generic keywords like 'left', 'right', 'ease-in' etc
    GenericKeyword(String),
    /// Quoted string that indicates css property
    Property(String),
    /// Functions like color(), length() etc
    Function(String, Option<Box<SyntaxComponent>>),
    /// Type definition like <length>, <color>, or quoted like <'background-color'>. Can include
    /// ranges like <percentage [0, 100]> etc.
    TypeDefinition(String, bool, RangeType),
    /// Inherit keyword
    Inherit,
    /// Initial keyword
    Initial,
    /// Unset keyword
    Unset,
    /// Literal character ',' or '/'
    Literal(String),
    /// CSS Value
    Value(CssValue),
    /// Group of components surrounded by []
    Group(Group),
    /// special unit() function case (todo: figure out if we need this special case)
    Unit(Option<f32>, Option<f32>, Vec<String>),
    /// Scalar elements (like: <integer>, <number, <percentage> etc)
    Scalar(String),
}

/// A value definition syntax structure. See https://developer.mozilla.org/en-US/docs/Web/CSS/Value_definition_syntax
pub(crate) struct CssSyntax {
    /// Source string of the syntax
    source: String,
}

impl CssSyntax {
    /// Generates a new syntax instance
    pub fn new(source: &str) -> Self {
        CssSyntax { source: source.to_string() }
    }

    /// Compiles the current syntax into a list of components or Err on compilation error
    pub fn compile(self) -> Result<CssSyntaxTree> {
        if self.source.is_empty() {
            return Ok(CssSyntaxTree::new(vec![]));
        }

        let p = parse(self.source.as_str());
        match p {
            Ok((input, components)) => {
                if !input.trim().is_empty() {
                    return Err(Error::CssCompile(
                        format!("Failed to parse all input (left: '{}')", input)
                    ).into());
                }
                Ok(CssSyntaxTree::new(vec![components]))
            }
            Err(err) => Err(Error::CssCompile(err.to_string()).into()),
        }
    }
}

/// Converts a list of components into either a single value, or a list if there are multiple values.
fn value_or_list(list: Vec<SyntaxComponent>, combinator: GroupCombinators) -> SyntaxComponent {
    if list.len() == 1 {
        return list.into_iter().next().unwrap();
    }

    SyntaxComponent::new(
        SyntaxComponentType::Group(Group {
            components: list.clone(),
            combinator,
        }),
        SyntaxComponentMultiplier::Once,
    )
}

/// Parse a unit input
fn parse_unit(input: &str) -> IResult<&str, SyntaxComponentType> {
    let (input, value) = float(input)?;
    let (input, suffix) = opt(alpha1)(input)?;

    if suffix.is_none() {
        return if value == 0.0 {
            // 0 is a special case as it doesn't need a unit
            Ok((input, SyntaxComponentType::Value(CssValue::Number(0.0))))
        } else {
            Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Alpha,
            )))
        };
    }

    Ok((
        input,
        SyntaxComponentType::Value(CssValue::Unit(value, suffix.unwrap().to_string())),
    ))
}

/// Removes preceding whitespace from a parser
fn ws<'a, F: 'a, O>(inner: F) -> impl FnMut(&'a str) -> IResult<&'a str, O>
    where
        F: FnMut(&'a str) -> IResult<&'a str, O>,
{
    delimited(multispace0, inner, multispace0)
    // preceded(multispace0, inner)
}

/// Parse a keyword (alphanumeric characters and dashes)
fn parse_keyword(input: &str) -> IResult<&str, &str> {
    let alpha_or_dash = alt((alphanumeric1, recognize(many1(one_of("-")))));
    recognize(fold_many1(alpha_or_dash, || (), |_, _| ()))(input)
}

/// Parse an integer
fn integer(input: &str) -> IResult<&str, u32> {
    map_res(digit0, |s: &str| s.parse::<u32>())(input)
}

fn parse_curly_braces_multiplier(input: &str) -> IResult<&str, SyntaxComponentMultiplier> {
    let range = alt((
        separated_pair(ws(integer), ws(tag(",")), ws(integer)),
        map(ws(integer), |num| (num, num)),
    ));
    let (input, range) = delimited(tag("{"), range, tag("}"))(input)?;

    Ok((
        input,
        SyntaxComponentMultiplier::Between(range.0 as usize, range.1 as usize),
    ))
}

fn parse_comma_separated_multiplier(input: &str) -> IResult<&str, SyntaxComponentMultiplier> {

    let range = alt((
        separated_pair(ws(integer), ws(tag(",")), ws(integer)),
        map(ws(integer), |num| (num, num)),
    ));

    let (input, minmax) = alt((
        map(delimited(ws(tag("#{")), range, ws(tag("}"))), |(min, max)| (min, max)),
        map(ws(tag("#")), |_| (1, 1)),
    ))(input)?;

    Ok((
        input,
        SyntaxComponentMultiplier::CommaSeparatedRepeat(
            minmax.0 as usize,
            minmax.1 as usize,
        ),
    ))
}

/// Parses any optional multipliers for a group
fn parse_multipliers(input: &str) -> IResult<&str, SyntaxComponentMultiplier> {
    debug_print!("Parsing multipliers: {}", input);

    let (input, multiplier) = opt(alt((
        map(tag("*"), |_| SyntaxComponentMultiplier::ZeroOrMore),
        map(tag("+"), |_| SyntaxComponentMultiplier::OneOrMore),
        map(tag("?"), |_| SyntaxComponentMultiplier::Optional),
        map(tag("!"), |_| SyntaxComponentMultiplier::AtLeastOneValue),
        parse_comma_separated_multiplier,
        parse_curly_braces_multiplier,
    )))(input)?;

    Ok((input, multiplier.unwrap_or(SyntaxComponentMultiplier::Once)))
}

/// Parse a group ([])
fn parse_group(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing group: {}", input);

    let (input, components) = delimited(ws(tag("[")), parse_component_list, ws(tag("]")))(input)?;

    let group = Group {
        components: vec![components],
        combinator: GroupCombinators::Juxtaposition,
    };

    debug_print!("<- Parsed group: {:#?}", group);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, SyntaxComponentType::Group(group)))
}

fn parse_component_singlebar_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component singlebar list: {}", input);

    let (input, list) = separated_list1(ws(tag("|")), parse_component)(input)?;
    let c = value_or_list(list, GroupCombinators::ExactlyOne);

    debug_print!("<- parse_component_singlebar_list: {:#?}", c);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, c))
}

fn parse_component_doublebar_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component doublebar list: {}", input);

    let (input, list) = separated_list1(ws(tag("||")), parse_component_singlebar_list)(input)?;
    let c = value_or_list(list, GroupCombinators::AtLeastOneAnyOrder);

    debug_print!("<- parse_component_doublebar_list: {:#?}", c);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, c))
}

fn parse_component_doubleampersand_list(input: &str) -> IResult<&str, SyntaxComponent> {
    let (input, list) = separated_list1(ws(tag("&&")), parse_component_doublebar_list)(input)?;

    let c = value_or_list(list, GroupCombinators::AllAnyOrder);

    debug_print!("<- parse_component_doubleampersand_list: {:#?}", c);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, c))
}


fn is_custom_separator(c: char) -> bool {
    if c == ',' {
        return false;
    }

    c == '|' || c == '&'
}

fn custom_separated_list_2(input: &str) -> IResult<&str, Vec<SyntaxComponent>> {
    debug_print!("Parsing custom separated list: {}", input);

    let mut res = Vec::new();

    let mut input = input;

    // Parser the first element
    match parse_component_doubleampersand_list(input) {
        Err(e) => return Err(e),
        Ok((input1, o)) => {
            res.push(o);
            input = input1;
        }
    }

    loop {
        if input.is_empty() {
            return Ok((input, res));
        }

        // A separator is:
        // - a space character followed by a comma
        // - a comma
        // - a space character followed by a | or & or [ or ]

        let (input1, _) = take_while(|c| is_custom_separator(c) || c.is_whitespace())(input)?;
        let (input1, _) = take_while(|c: char| c.is_whitespace())(input1)?;

        if input1.is_empty() {
            return Ok((input, res));
        }


        match parse_component_doubleampersand_list(input1) {
            Err(Err::Error(_)) => return Ok((input, res)),
            Err(e) => return Err(e),
            Ok((input2, o)) => {
                res.push(o);
                input = input2;
            }
        }
    }
}

fn parse_component_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component list: {}", input);

    let (input, list) = custom_separated_list_2(input)?;
    let c = value_or_list(list, GroupCombinators::Juxtaposition);

    debug_print!("<- parse_component_list: {:#?}", c);
    debug_print!("<- Remaining: {:#?}", input);
    Ok((input, c))
}

fn int_as_float(input: &str) -> IResult<&str, f32> {
    map(integer, |i| i as f32)(input)
}

fn parse_unit_inner(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing parse_unit_inner: {}", input);

    let single_int = map(integer, |i| (Some(i as f32), None));
    let paired_int = separated_pair(opt(int_as_float), tag(".."), opt(int_as_float));

    let (input, range) = opt(alt((paired_int, single_int)))(input)?;

    // Find any optional suffixes
    let (input, _) = multispace0(input)?;
    let (input, suffixes) = opt(separated_list0(ws(tag("|")), alpha1))(input)?;

    if suffixes.is_none() {
        // No suffixes, just a range
        return Ok((
            input,
            SyntaxComponentType::Unit(
                range.unwrap_or((None, None)).0,
                range.unwrap_or((None, None)).1,
                vec![],
            ),
        ));
    }

    // Convert the suffixes to a vector of strings
    let suffixes: Vec<String> = suffixes.unwrap().iter().map(|s| s.to_string()).collect();
    Ok((
        input,
        SyntaxComponentType::Unit(
            range.unwrap_or((None, None)).0,
            range.unwrap_or((None, None)).1,
            suffixes,
        ),
    ))
}

fn parse_unit_function(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing unit_function: {}", input);
    let (input, unit) = delimited(tag("unit("), parse_unit_inner, tag(")"))(input)?;

    Ok((input, unit))
}

fn parse_function(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing function: {}", input);

    let empty_arglist = ws(tag("()"));
    let arglist = delimited(ws(tag("(")), ws(parse_component_list), ws(tag(")")));

    let (input, name) = parse_keyword(input)?;
    let (input, arglist) = alt((
        map(empty_arglist, |_| None),
        map(arglist, |c| Some(Box::new(c))),
    ))(input)?;

    Ok((input, SyntaxComponentType::Function(name.to_string(), arglist)))
}

fn parse_property(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing property: {}", input);

    let (input, property) = delimited(
        tag("'"),
        map(parse_keyword, |s: &str| SyntaxComponentType::Property(s.to_string())),
        tag("'"),
    )(input)?;

    Ok((input, property))
}

fn parse_generic_keyword(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing generic keyword: {}", input);

    map(parse_keyword, |s: &str| {
        SyntaxComponentType::GenericKeyword(s.to_string())
    })(input)
}

/// Parses an infinity symbol and returns NumberOrInfinity::Infinity
fn parse_infinity(input: &str) -> IResult<&str, NumberOrInfinity> {
    alt((
        map(tag_no_case("inf"), |_| NumberOrInfinity::Infinity),
        map(tag_no_case("-inf"), |_| NumberOrInfinity::NegativeInfinity),
    ))(input)
}

/// Parses an integer (signed or unsigned) and returns NumberOrInfinity::FiniteI64, or errors when the integer is invalid
fn parse_signed_integer(input: &str) -> IResult<&str, NumberOrInfinity> {
    map_res(
        pair(opt(char('-')), digit1),
        |(sign, digits): (Option<char>, &str)| {
            let neg_multiplier = if sign == Some('-') { -1 } else { 1 };
            let num = digits.parse::<i64>().map(|num| num * neg_multiplier);
            if num.is_ok() {
                Ok(NumberOrInfinity::FiniteI64(num.unwrap()))
            } else {
                Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit,
                )))
            }
        },
    )(input)
}

/// Parses a range for a type definition  (ie: the square bracket part of: <function [1, 10]>)
fn typedef_range(input: &str) -> IResult<&str, RangeType> {
    let range = separated_pair(
        opt(ws(alt((parse_infinity, parse_signed_integer)))),
        tag(","),
        opt(ws(alt((parse_infinity, parse_signed_integer)))),
    );

    let range = map(range, |(min, max)| RangeType {
        min: min.unwrap_or(NumberOrInfinity::None),
        max: max.unwrap_or(NumberOrInfinity::None),
    });

    let (input, r) = delimited(ws(tag("[")), range, ws(tag("]")))(input)?;

    Ok((input, r))
}


fn keyword_or_function(input: &str) -> IResult<&str, &str> {
    recognize(pair(parse_keyword, opt(tag("()"))))(input)
}

fn parse_typedef(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing typedef: {}", input);

    let (input, (name, quoted, range)) = delimited(
        ws(tag("<")),
        alt((
            map(pair(keyword_or_function, opt(typedef_range)), |(name, range)| (name, false, range)),
            map(pair(delimited(ws(tag("'")), keyword_or_function, ws(tag("'"))), opt(typedef_range)), |(name, range)| (name, true, range)),
        )),
        ws(tag(">")),
    )(input)?;

    Ok((input, SyntaxComponentType::TypeDefinition(name.to_string(), quoted, range.unwrap_or(RangeType::empty()))))
}


fn parse_specific_keyword(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing specific keyword: {}", input);

    alt((
        map(tag("inherit"), |_| SyntaxComponentType::Inherit),
        map(tag("initial"), |_| SyntaxComponentType::Initial),
        map(tag("unset"), |_| SyntaxComponentType::Unset),
    ))(input)
}

fn parse_literal(input: &str) -> IResult<&str, SyntaxComponentType> {
    debug_print!("Parsing literal: {}", input);

    alt((
        map(ws(tag("/")), |_| SyntaxComponentType::Literal("/".to_string())),
        map(ws(tag(",")), |_| SyntaxComponentType::Literal(",".to_string())),
        map(delimited(tag("'"), take_while(|c| c != '\''), tag("'")), |s: &str| SyntaxComponentType::Literal(s.to_string())),
    ))(input)
}

fn parse_component(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component: {}", input);

    let (input, component_type) = alt((
        parse_unit_function,
        parse_function,
        parse_property,
        parse_specific_keyword,
        parse_literal,
        parse_group,
        parse_unit,
        parse_typedef,
        parse_generic_keyword, // This is more of a catch-all
    ))(input)?;
    let (input, multipliers) = parse_multipliers(input)?;

    debug_print!(
        "<- Parsed component_type: {:#?} {}",
        component_type,
        multipliers
    );

    let component = SyntaxComponent {
        type_: component_type,
        multipliers,
    };

    Ok((input, component))
}

fn parse(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing: {}", input);
    let (input, result) = preceded(multispace0, parse_component_list)(input)?;
    debug_print!("<- Parsed: {:#?}", result);
    Ok((input, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_definitions::get_css_definitions;

    #[test]
    fn test_compile_empty() {
        assert!(CssSyntax::new("").compile().is_ok());
    }

    #[test]
    fn test_compile_all_definitions() {
        // Fetching the definitions will automatically compile all definitions on the first run
        let defs = get_css_definitions();
        assert!(!defs.is_empty());
    }

    #[test]
    fn test_generic() {
        let parts = CssSyntax::new("ease-in").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("ease-in".to_string()),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("color").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::Once,
            )])
        );
    }

    #[test]
    fn test_unit() {
        let parts = CssSyntax::new("unit()").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(None, None, vec![]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(None, None, vec!["khz".into()]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(ms|s)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(None, None, vec!["ms".into(), "s".into()]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(10..10000 khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(Some(10.0), Some(10000.0), vec!["khz".into()]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(0.. ms|s)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(Some(0.0), None, vec!["ms".into(), "s".into()]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(..10000 khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(None, Some(10000.0), vec!["khz".into()]),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unit(10..10000)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unit(Some(10.0), Some(10000.0), vec![]),
                SyntaxComponentMultiplier::Once,
            )])
        );
    }

    #[test]
    fn test_multipliers() {
        let parts = CssSyntax::new("color").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("color*").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::ZeroOrMore,
            )])
        );

        let parts = CssSyntax::new("color+").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::OneOrMore,
            )])
        );

        let parts = CssSyntax::new("color?").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::Optional,
            )])
        );

        let parts = CssSyntax::new("color{3,5}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::Between(3, 5),
            )])
        );

        let parts = CssSyntax::new("color#").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::CommaSeparatedRepeat(1, 1),
            )])
        );

        let parts = CssSyntax::new("color#{3,6}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::CommaSeparatedRepeat(3, 6),
            )])
        );

        let parts = CssSyntax::new("color!").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::GenericKeyword("color".to_string()),
                SyntaxComponentMultiplier::AtLeastOneValue,
            )])
        );
    }

    #[test]
    fn test_function() {
        let parts = CssSyntax::new("length(){2,4}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Function("length".to_string(), None),
                SyntaxComponentMultiplier::Between(2, 4),
            )])
        );

        let parts = CssSyntax::new("color()").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Function("color".to_string(), None),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("color(top)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Function("color".to_string(), Some(Box::new(SyntaxComponent::new(
                    SyntaxComponentType::GenericKeyword("top".to_string()),
                    SyntaxComponentMultiplier::Once,
                )))),
                SyntaxComponentMultiplier::Once,
            )])
        );
    }

    #[test]
    fn test_literal() {
        let parts = CssSyntax::new("/").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Literal("/".to_string()),
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new(",").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Literal(",".to_string()),
                SyntaxComponentMultiplier::Once,
            )])
        );
    }

    #[test]
    fn test_special_keywords() {
        let parts = CssSyntax::new("unset").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unset,
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("initial").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Initial,
                SyntaxComponentMultiplier::Once,
            )])
        );

        let parts = CssSyntax::new("unset").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::new(
                SyntaxComponentType::Unset,
                SyntaxComponentMultiplier::Once,
            )])
        );
    }

    #[test]
    fn test_compile_unit() {
        let parts = CssSyntax::new("10px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::Value(CssValue::Unit(10.0, "px".to_string())),
                SyntaxComponentMultiplier::Once,
            )]
        );

        let parts = CssSyntax::new("10.43px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::Value(CssValue::Unit(10.43, "px".to_string())),
                SyntaxComponentMultiplier::Once,
            )]
        );

        let parts = CssSyntax::new("-10.43px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::Value(CssValue::Unit(-10.43, "px".to_string())),
                SyntaxComponentMultiplier::Once,
            )]
        );

        let parts = CssSyntax::new("0").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::Value(CssValue::Number(0.0)),
                SyntaxComponentMultiplier::Once,
            )]
        );
    }

    #[test]
    fn test_compile_typedef() {
        let parts = CssSyntax::new("<foo> | <bar()> | <'quoted'>").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::Group(
                    Group {
                        combinator: GroupCombinators::ExactlyOne,
                        components: vec![
                            SyntaxComponent::new(
                                SyntaxComponentType::TypeDefinition("foo".to_string(), false, RangeType::empty()),
                                SyntaxComponentMultiplier::Once,
                            ),
                            SyntaxComponent::new(
                                SyntaxComponentType::TypeDefinition("bar()".to_string(), false, RangeType::empty()),
                                SyntaxComponentMultiplier::Once,
                            ),
                            SyntaxComponent::new(
                                SyntaxComponentType::TypeDefinition("quoted".to_string(), true, RangeType::empty()),
                                SyntaxComponentMultiplier::Once,
                            ),
                        ],
                    }
                ),
                SyntaxComponentMultiplier::Once,
            )]
        );

        let parts = CssSyntax::new("<foo>#").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::new(
                SyntaxComponentType::TypeDefinition("foo".to_string(), false, RangeType::empty()),
                SyntaxComponentMultiplier::CommaSeparatedRepeat(1, 1),
            )]
        );

    }

    #[test]
    fn test_parse_unit() {
        assert!(parse_unit("10px").is_ok());
        assert!(parse_unit("0").is_ok());
        assert!(parse_unit("52").is_err());
        assert!(parse_unit("0.0").is_ok());
        assert!(parse_unit("0.1px").is_ok());
        assert!(parse_unit("0.1foobar").is_ok());
    }

    #[test]
    fn test_precedence() {
        let c = CssSyntax::new("left | right").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::ExactlyOne,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left | right && top").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left && right | top").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left && right || top").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::AtLeastOneAnyOrder,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left || right | top").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AtLeastOneAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left | right || top").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AtLeastOneAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left | right || top && bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::AtLeastOneAnyOrder,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::Group(Group {
                            combinator: GroupCombinators::ExactlyOne,
                            components: vec![
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                            ],
                        }), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left || right | top && bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::AtLeastOneAnyOrder,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::Group(Group {
                            combinator: GroupCombinators::ExactlyOne,
                            components: vec![
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                            ],
                        }), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left && right || top | bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AllAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::AtLeastOneAnyOrder,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::Group(Group {
                            combinator: GroupCombinators::ExactlyOne,
                            components: vec![
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
                            ],
                        }), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left  right || top | bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::Juxtaposition,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::AtLeastOneAnyOrder,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::Group(Group {
                            combinator: GroupCombinators::ExactlyOne,
                            components: vec![
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
                            ],
                        }), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left | right || top | bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AtLeastOneAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left || right | top || bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::AtLeastOneAnyOrder,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));

        let c = CssSyntax::new("left right | top bottom").compile().unwrap();
        assert_eq!(c.components[0].type_, SyntaxComponentType::Group(Group {
            combinator: GroupCombinators::Juxtaposition,
            components: vec![
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("left".to_string()), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::Group(Group {
                    combinator: GroupCombinators::ExactlyOne,
                    components: vec![
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("right".to_string()), SyntaxComponentMultiplier::Once),
                        SyntaxComponent::new(SyntaxComponentType::GenericKeyword("top".to_string()), SyntaxComponentMultiplier::Once),
                    ],
                }), SyntaxComponentMultiplier::Once),
                SyntaxComponent::new(SyntaxComponentType::GenericKeyword("bottom".to_string()), SyntaxComponentMultiplier::Once),
            ],
        }));
    }

    #[test]
    fn test_typedef_ranges() {
        let c = CssSyntax::new("<function [1, 2]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::FiniteI64(1),
                           max: NumberOrInfinity::FiniteI64(2),
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));

        let c = CssSyntax::new("<function>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::None,
                           max: NumberOrInfinity::None,
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));

        let c = CssSyntax::new("<function [1,]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::FiniteI64(1),
                           max: NumberOrInfinity::None,
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));

        let c = CssSyntax::new("<function [,1]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::None,
                           max: NumberOrInfinity::FiniteI64(1),
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));

        let c = CssSyntax::new("<function [-360,360]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::FiniteI64(-360),
                           max: NumberOrInfinity::FiniteI64(360),
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));

        let c = CssSyntax::new("<function [0,inf]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::FiniteI64(0),
                           max: NumberOrInfinity::Infinity,
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));
        let c = CssSyntax::new("<function [-inf, 0]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::NegativeInfinity,
                           max: NumberOrInfinity::FiniteI64(0),
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));
        let c = CssSyntax::new("<function [-inf,inf]>").compile().unwrap();
        assert_eq!(c.components[0],
                   SyntaxComponent::new(SyntaxComponentType::TypeDefinition(
                       "function".to_string(),
                       false,
                       RangeType {
                           min: NumberOrInfinity::NegativeInfinity,
                           max: NumberOrInfinity::Infinity,
                       },
                   ), SyntaxComponentMultiplier::Once,
                   ));
    }


    #[test]
    fn test_specific_precedence_configurations() {
        // let c = CssSyntax::new("rgb( [ <number> | <percentage> | none]{3} [ / [<alpha-value> | none] ]? )").compile();
        // let c = CssSyntax::new("<percentage>#{3}").compile();
        // dbg!(&c);
        // return;

        assert!(CssSyntax::new("le, ri ,co , bt,tp").compile().is_ok());
        assert!(CssSyntax::new("left | right | center && top").compile().is_ok());
        assert!(CssSyntax::new("left , right color()").compile().is_ok());
        assert!(CssSyntax::new("left , right color() ").compile().is_ok());
        assert!(CssSyntax::new("le, ri ,co , bt,tp").compile().is_ok());
        assert!(CssSyntax::new("left, right color()").compile().is_ok());
        assert!(CssSyntax::new("left | right | center && top").compile().is_ok());
        assert!(CssSyntax::new("left | right | center && top <length>").compile().is_ok());
        assert!(CssSyntax::new("[ [ <length-percentage>? ]]").compile().is_ok());
        assert!(CssSyntax::new("[ [ center | [ top | bottom ]  ]]").compile().is_ok());
        assert!(CssSyntax::new("[ <length-percentage>? ]").compile().is_ok());
        assert!(CssSyntax::new("[ center <length-percentage>? ]").compile().is_ok());
        assert!(CssSyntax::new("center | [ top | bottom ] <length-percentage>").compile().is_ok());
        assert!(CssSyntax::new("[ center | [ top | bottom ] <length-percentage> ]").compile().is_ok());
        assert!(CssSyntax::new("[ center | [ top | bottom ] <length-percentage>? ]").compile().is_ok());
        assert!(CssSyntax::new("[ [ center | [ top | bottom ] <length-percentage>? ]]").compile().is_ok());
        assert!(CssSyntax::new("[ [ top | center | bottom | <length-percentage> ]| [ center | [ left | right ] <length-percentage>? ] && [ center | [ top | bottom ] <length-percentage>? ]]").compile().is_ok());
        assert!(CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ]| [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ]| [ center | [ left | right ] <length-percentage>? ] && [ center | [ top | bottom ] <length-percentage>? ]]").compile().is_ok());
    }
}
