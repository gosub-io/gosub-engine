use std::fmt::{Debug, Display, Formatter};

use nom::branch::alt;
use nom::bytes::complete::{tag, tag_no_case, take_while};
use nom::character::complete::{
    alpha1, alphanumeric1, char, digit0, digit1, multispace0, one_of, space0,
};
use nom::combinator::{map, map_res, opt, recognize};
use nom::multi::{fold_many1, many0, many1, separated_list0, separated_list1};
use nom::number::complete::float;
use nom::sequence::{delimited, pair, preceded, separated_pair, tuple};
use nom::Err;
use nom::IResult;

use gosub_css3::stylesheet::CssValue;
use gosub_shared::types::Result;

use crate::errors::Error;
use crate::syntax_matcher::CssSyntaxTree;

// When debugging the parser, it's nice to have some additional information ready. This should maybe
// be inside a cfg setting, but for now (un)commenting the appropriate line is good enough.
macro_rules! debug_print {
    // ($($x:tt)*) => { println!($($x)*) }
    ($($x:tt)*) => {{}};
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
            SyntaxComponentMultiplier::CommaSeparatedRepeat(min, max) => {
                write!(f, "#{{{}, {}}}", min, max)
            }
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

/// Syntax components. These are the elements that make up the css declaration syntax.
#[derive(PartialEq, Debug, Clone)]
pub enum SyntaxComponent {
    /// Generic keyword strings like 'left', 'right', 'ease-in' etc
    GenericKeyword {
        keyword: String,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Quoted string that indicates css property
    Property {
        property: String,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Functions like color(), length() etc
    Function {
        name: String,
        arguments: Option<Box<SyntaxComponent>>,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Internal data definition like <length>, <color>, or quoted like <'background-color'>. Can include
    /// ranges like <percentage [0, 100]> etc.
    Definition {
        datatype: String,
        quoted: bool,
        range: RangeType,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Inherit keyword
    Inherit {
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Initial keyword
    Initial {
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Unset keyword
    Unset {
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Literal character ',' or '/'
    Literal {
        literal: String,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// CSS Value
    Value {
        value: CssValue,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// Group of components surrounded by []
    Group {
        components: Vec<SyntaxComponent>,
        combinator: GroupCombinators,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    /// special unit() function case (@todo: figure out if we need this special case)
    Unit {
        from: Option<f32>,
        to: Option<f32>,
        unit: Vec<String>,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
    Builtin {
        datatype: String,
        multipliers: Vec<SyntaxComponentMultiplier>,
    },
}

impl SyntaxComponent {
    pub fn is_group(&self) -> bool {
        matches!(self, SyntaxComponent::Group { .. })
    }

    pub fn get_multipliers(&self) -> Vec<SyntaxComponentMultiplier> {
        match self {
            SyntaxComponent::Group { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Function { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Property { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::GenericKeyword { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Definition { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Unit { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Literal { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Inherit { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Initial { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Unset { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Value { multipliers, .. } => multipliers.clone(),
            SyntaxComponent::Builtin { multipliers, .. } => multipliers.clone(),
        }
    }

    pub fn update_multipliers(&mut self, new_multipliers: Vec<SyntaxComponentMultiplier>) {
        match self {
            SyntaxComponent::Group { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Function { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Property { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::GenericKeyword { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Definition { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Unit { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Literal { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Inherit { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Initial { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Unset { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Value { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
            SyntaxComponent::Builtin { multipliers, .. } => {
                *multipliers = new_multipliers;
            }
        }
    }
}

/// A value definition syntax structure. See https://developer.mozilla.org/en-US/docs/Web/CSS/Value_definition_syntax
pub(crate) struct CssSyntax {
    /// Source string of the syntax
    source: String,
}

impl CssSyntax {
    /// Generates a new syntax instance
    pub fn new(source: &str) -> Self {
        CssSyntax {
            source: source.to_string(),
        }
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
                    return Err(Error::CssCompile(format!(
                        "Failed to parse all input (left: '{}')",
                        input
                    ))
                    .into());
                }
                Ok(CssSyntaxTree::new(vec![components]))
            }
            Err(err) => Err(Error::CssCompile(err.to_string()).into()),
        }
    }
}

/// Parse a unit input
fn parse_unit(input: &str) -> IResult<&str, SyntaxComponent> {
    let (input, value) = float(input)?;
    let (input, suffix) = opt(alpha1)(input)?;

    if suffix.is_none() {
        return if value == 0.0 {
            // 0 is a special case as it doesn't need a unit
            Ok((
                input,
                SyntaxComponent::Value {
                    value: CssValue::Zero,
                    multipliers: vec![SyntaxComponentMultiplier::Once],
                },
            ))
        } else {
            Err(Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Alpha,
            )))
        };
    }

    Ok((
        input,
        SyntaxComponent::Value {
            value: CssValue::Unit(value, suffix.unwrap().to_string()),
            multipliers: vec![SyntaxComponentMultiplier::Once],
        },
    ))
}

/// Removes preceding whitespace from a parser
fn ws<'a, F, O>(inner: F) -> impl FnMut(&'a str) -> IResult<&'a str, O>
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
        map(
            delimited(ws(tag("#{")), range, ws(tag("}"))),
            |(min, max)| (min, max),
        ),
        // No range means one or more values
        map(ws(tag("#")), |_| (1, u32::MAX)),
    ))(input)?;

    Ok((
        input,
        SyntaxComponentMultiplier::CommaSeparatedRepeat(minmax.0 as usize, minmax.1 as usize),
    ))
}

/// Parses any optional multipliers for a group
fn parse_multipliers(input: &str) -> IResult<&str, Vec<SyntaxComponentMultiplier>> {
    debug_print!("Parsing multipliers: '{}'", input);

    let (input, multipliers) = many0(alt((
        map(tag("*"), |_| SyntaxComponentMultiplier::ZeroOrMore),
        map(tag("+"), |_| SyntaxComponentMultiplier::OneOrMore),
        map(tag("?"), |_| SyntaxComponentMultiplier::Optional),
        map(tag("!"), |_| SyntaxComponentMultiplier::AtLeastOneValue),
        parse_comma_separated_multiplier,
        parse_curly_braces_multiplier,
    )))(input)?;

    if multipliers.is_empty() {
        return Ok((input, vec![SyntaxComponentMultiplier::Once]));
    }

    Ok((input, multipliers))
}

/// Parse a group ([])
fn parse_group(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing group: {}", input);

    let (input, components) =
        delimited(ws(tag("[")), parse_component_singlebar_list, ws(tag("]")))(input)?;

    Ok((input, components))
}

fn parse_component_singlebar_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component singlebar list: {}", input);

    let (input, components) = separated_list1(ws(tag("|")), parse_component_doublebar_list)(input)?;

    if components.len() == 1 {
        return Ok((input, components[0].clone()));
    }

    let group = SyntaxComponent::Group {
        components,
        combinator: GroupCombinators::ExactlyOne,
        multipliers: vec![SyntaxComponentMultiplier::Once],
    };

    debug_print!("<- parse_component_singlebar_list: {:#?}", group);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, group))
}

fn parse_component_doublebar_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component doublebar list: {}", input);

    let (input, components) =
        separated_list1(ws(tag("||")), parse_component_doubleampersand_list)(input)?;

    if components.len() == 1 {
        return Ok((input, components[0].clone()));
    }

    let group = SyntaxComponent::Group {
        components,
        combinator: GroupCombinators::AtLeastOneAnyOrder,
        multipliers: vec![SyntaxComponentMultiplier::Once],
    };

    debug_print!("<- parse_component_doublebar_list: {:#?}", group);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, group))
}

fn parse_component_doubleampersand_list(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component doubleampersand list: {}", input);

    let (input, components) =
        separated_list1(ws(tag("&&")), parse_component_juxtaposition_list)(input)?;

    if components.len() == 1 {
        return Ok((input, components[0].clone()));
    }

    let group = SyntaxComponent::Group {
        components,
        combinator: GroupCombinators::AllAnyOrder,
        multipliers: vec![SyntaxComponentMultiplier::Once],
    };

    debug_print!("<- parse_component_doubleampersand_list: {:#?}", group);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, group))
}

/// Find a separator for juxtaposition group. Or actually, find a separator that isn't for
/// a juxtaposition group. If we remove the spaces, then we can check for the next non-space
/// character. If it's a |, or a &, it's part of another group separator (|, || or &&). OR,
/// as another case, when we find a ], we are at the end of a group, so that will also be the
/// end of this juxtaposition group. The only extra check we need to do if we reach the end
/// of the input. If so, then we are also at the end of the group.
///
/// This function will return the input with leading spaces removed, but keeps the next separator
/// or char for the parser to pick up. Finally, it will return a boolean true if it's not the end
/// of the juxtaposition group, or false when the end is found.
fn juxtaseparator(input: &str) -> IResult<&str, bool> {
    let (input, _) = space0(input)?;

    // Special case when there is no more input
    if input.is_empty() {
        return Ok((input, false));
    }

    // Any of these chars means we found the end of the juxaposition group
    let (_, end_of_group) = opt(alt((
        char(']'), // End of a group
        char(')'), // End of a function argument list
        char('|'), // Start of a separator for non-juxtaposition group
        char('&'), // Start of a separator for non-juxtaposition group
    )))(input)?;

    // If we didn't find any of the above chars, we return true, as we seem to have found a
    // next juxtaposition element.
    Ok((input, end_of_group.is_none()))
}

// We need to use a custom separated_list1 for this, as that function MUST capture a separator
// (even if it's just a space). Since juxtaposition is only spaces, and we remove them anyway,
// we cannot use that function (it will fail when the output length is the same as the input length)
// We might be able to rewrite this function.
fn juxtaposition_or_separated_list(input: &str) -> IResult<&str, Vec<SyntaxComponent>> {
    let mut elements = Vec::new();
    let mut input = input;

    loop {
        let (next_input, _) = space0(input)?;

        // Parse an element
        let (next_input, elem) = parse_component(next_input)?;
        elements.push(elem);

        // Check for a separator
        let result = juxtaseparator(next_input);

        // If errored, we return what we've got
        if result.is_err() {
            return Ok((next_input, elements));
        }

        // If found, the sep boolean determines if we are done or not.
        let (next_input, sep) = result.unwrap();
        if !sep {
            return Ok((next_input, elements));
        }

        input = next_input;
    }
}

fn parse_component_juxtaposition_list(input: &str) -> IResult<&str, SyntaxComponent> {
    let (input, components) = juxtaposition_or_separated_list(input)?;
    if components.len() == 1 {
        return Ok((input, components[0].clone()));
    }

    let group = SyntaxComponent::Group {
        components,
        combinator: GroupCombinators::Juxtaposition,
        multipliers: vec![SyntaxComponentMultiplier::Once],
    };

    debug_print!("<- parse_component_juxtaposition_list: {:#?}", group);
    debug_print!("<- Remaining input: '{}'", input);
    Ok((input, group))
}

fn int_as_float(input: &str) -> IResult<&str, f32> {
    map(integer, |i| i as f32)(input)
}

fn parse_unit_inner(input: &str) -> IResult<&str, SyntaxComponent> {
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
            SyntaxComponent::Unit {
                from: range.unwrap_or((None, None)).0,
                to: range.unwrap_or((None, None)).1,
                unit: vec![],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            },
        ));
    }

    // Convert the suffixes to a vector of strings
    let suffixes: Vec<String> = suffixes.unwrap().iter().map(|s| s.to_string()).collect();
    Ok((
        input,
        SyntaxComponent::Unit {
            from: range.unwrap_or((None, None)).0,
            to: range.unwrap_or((None, None)).1,
            unit: suffixes,
            multipliers: vec![SyntaxComponentMultiplier::Once],
        },
    ))
}

fn parse_unit_function(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing unit_function: {}", input);
    let (input, unit) = delimited(tag("unit("), parse_unit_inner, tag(")"))(input)?;

    Ok((input, unit))
}

fn parse_function(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing function: {}", input);

    let empty_arglist = delimited(
        tuple((space0, char('('), space0)),
        space0,
        tuple((space0, char(')'), space0)),
    );
    let arglist = delimited(
        ws(tag("(")),
        ws(parse_component_singlebar_list),
        ws(tag(")")),
    );

    let (input, name) = parse_keyword(input)?;
    let (input, arglist) = alt((map(empty_arglist, |_| None), map(arglist, Some)))(input)?;

    match arglist {
        Some(arglist) => Ok((
            input,
            SyntaxComponent::Function {
                name: name.to_string(),
                arguments: Some(Box::new(arglist)),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            },
        )),
        None => Ok((
            input,
            SyntaxComponent::Function {
                name: name.to_string(),
                arguments: None,
                multipliers: vec![SyntaxComponentMultiplier::Once],
            },
        )),
    }
}

fn parse_property(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing property: {}", input);

    let (input, property) = delimited(
        tag("'"),
        map(parse_keyword, |s: &str| SyntaxComponent::Property {
            property: s.to_string(),
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
        tag("'"),
    )(input)?;

    Ok((input, property))
}

fn parse_generic_keyword(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing generic keyword: '{}'", input);

    map(parse_keyword, |s: &str| SyntaxComponent::GenericKeyword {
        keyword: s.to_string(),
        multipliers: vec![SyntaxComponentMultiplier::Once],
    })(input)
}

/// Parses an infinity symbol and returns NumberOrInfinity::Infinity
fn parse_infinity(input: &str) -> IResult<&str, NumberOrInfinity> {
    alt((
        map(tag_no_case("inf"), |_| NumberOrInfinity::Infinity),
        map(tag_no_case("∞"), |_| NumberOrInfinity::Infinity),
        map(tag_no_case("-inf"), |_| NumberOrInfinity::NegativeInfinity),
        map(tag_no_case("-∞"), |_| NumberOrInfinity::NegativeInfinity),
    ))(input)
}

/// Parses an integer (signed or unsigned) and returns NumberOrInfinity::FiniteI64, or errors when the integer is invalid
fn parse_signed_integer(input: &str) -> IResult<&str, NumberOrInfinity> {
    map_res(
        pair(opt(char('-')), digit1),
        |(sign, digits): (Option<char>, &str)| {
            let neg_multiplier = if sign == Some('-') { -1 } else { 1 };
            let num = digits.parse::<i64>().map(|num| num * neg_multiplier);
            if let Ok(num) = num {
                Ok(NumberOrInfinity::FiniteI64(num))
            } else {
                Err(nom::Err::Error(nom::error::Error::new(
                    input,
                    nom::error::ErrorKind::Digit,
                )))
            }
        },
    )(input)
}

fn parse_unit_range(input: &str) -> IResult<&str, NumberOrInfinity> {
    let out = parse_unit(input)?;
    if let SyntaxComponent::Value { value, .. } = out.1 {
        let val = match value {
            CssValue::Unit(v, _) => v as i64,
            CssValue::Zero => 0,
            CssValue::Number(v) => v as i64,
            v => {
                panic!("Invalid value: {:?}", v);
            }
        };

        let val = NumberOrInfinity::FiniteI64(val);

        return Ok((out.0, val));
    }

    todo!("parse_unit_rang is not implemented for non-values")
}

/// Parses a range for a type definition  (ie: the square bracket part of: <function [1, 10]>)
fn datatype_range(input: &str) -> IResult<&str, RangeType> {
    let range = separated_pair(
        opt(ws(alt((
            parse_infinity,
            parse_unit_range,
            parse_signed_integer,
        )))),
        tag(","),
        opt(ws(alt((
            parse_infinity,
            parse_unit_range,
            parse_signed_integer,
        )))),
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

fn parse_datatype(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing datatype: {}", input);

    let (input, (name, quoted, range)) = delimited(
        ws(tag("<")),
        alt((
            map(
                pair(keyword_or_function, opt(datatype_range)),
                |(name, range)| (name, false, range),
            ),
            map(
                pair(
                    delimited(ws(tag("'")), keyword_or_function, ws(tag("'"))),
                    opt(datatype_range),
                ),
                |(name, range)| (name, true, range),
            ),
        )),
        ws(tag(">")),
    )(input)?;

    Ok((
        input,
        SyntaxComponent::Definition {
            datatype: name.to_string(),
            quoted,
            range: range.unwrap_or(RangeType::empty()),
            multipliers: vec![SyntaxComponentMultiplier::Once],
        },
    ))
}

fn parse_specific_keyword(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing specific keyword: {}", input);

    alt((
        map(tag("inherit"), |_| SyntaxComponent::Inherit {
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
        map(tag("initial"), |_| SyntaxComponent::Initial {
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
        map(tag("unset"), |_| SyntaxComponent::Unset {
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
    ))(input)
}

fn parse_literal(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing literal: {}", input);

    alt((
        map(ws(tag("/")), |_| SyntaxComponent::Literal {
            literal: "/".to_string(),
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
        map(ws(tag(",")), |_| SyntaxComponent::Literal {
            literal: ",".to_string(),
            multipliers: vec![SyntaxComponentMultiplier::Once],
        }),
        map(
            delimited(tag("'"), take_while(|c| c != '\''), tag("'")),
            |s: &str| SyntaxComponent::Literal {
                literal: s.to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            },
        ),
    ))(input)
}

fn parse_component(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing component: {}", input);

    let (input, mut component) = alt((
        parse_unit_function,
        parse_function,
        parse_property,
        parse_specific_keyword,
        parse_literal,
        parse_group,
        parse_unit,
        parse_datatype,
        parse_generic_keyword, // This is more of a catch-all
    ))(input)?;
    let (input, multipliers) = parse_multipliers(input)?;

    component.update_multipliers(multipliers.clone());

    debug_print!("<- Parsed component_type: {:#?} {}", component, multipliers);

    Ok((input, component))
}

fn parse(input: &str) -> IResult<&str, SyntaxComponent> {
    debug_print!("Parsing: {}", input);
    let (input, component) = preceded(multispace0, parse_component_singlebar_list)(input)?;
    debug_print!("<- Parsed: {:#?}", component);

    Ok((input, component))
}

#[cfg(test)]
mod tests {
    use crate::property_definitions::get_css_definitions;

    use super::*;

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
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "ease-in".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("color").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".into(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );
    }

    #[test]
    fn test_unit() {
        let parts = CssSyntax::new("unit()").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: None,
                to: None,
                unit: vec![],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: None,
                to: None,
                unit: vec!["khz".into()],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(ms|s)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: None,
                to: None,
                unit: vec!["ms".into(), "s".into()],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(10..10000 khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: Some(10.0),
                to: Some(10000.0),
                unit: vec!["khz".into()],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(0.. ms|s)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: Some(0.0),
                to: None,
                unit: vec!["ms".into(), "s".into()],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(..10000 khz)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: None,
                to: Some(10000.0),
                unit: vec!["khz".into()],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unit(10..10000)").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unit {
                from: Some(10.0),
                to: Some(10000.0),
                unit: vec![],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );
    }

    #[test]
    fn test_multipliers() {
        let parts = CssSyntax::new("color").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("color*").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::ZeroOrMore],
            }])
        );

        let parts = CssSyntax::new("color+").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::OneOrMore],
            }])
        );

        let parts = CssSyntax::new("color?").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Optional],
            }])
        );

        let parts = CssSyntax::new("color{3,5}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Between(3, 5)],
            }])
        );

        let parts = CssSyntax::new("color#").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::CommaSeparatedRepeat(
                    1,
                    u32::MAX as usize
                )],
            }])
        );

        let parts = CssSyntax::new("color#{3,6}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::CommaSeparatedRepeat(3, 6)],
            }])
        );

        let parts = CssSyntax::new("color!").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "color".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::AtLeastOneValue],
            }])
        );
    }

    #[test]
    fn test_function() {
        let parts = CssSyntax::new("length(){2,4}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Function {
                name: "length".into(),
                arguments: None,
                multipliers: vec![SyntaxComponentMultiplier::Between(2, 4)],
            }])
        );

        let parts = CssSyntax::new("color()").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Function {
                name: "color".into(),
                arguments: None,
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("color(top?){2,4}").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Function {
                name: "color".into(),
                arguments: Some(Box::new(SyntaxComponent::GenericKeyword {
                    keyword: "top".into(),
                    multipliers: vec![SyntaxComponentMultiplier::Optional],
                })),
                multipliers: vec![SyntaxComponentMultiplier::Between(2, 4)],
            }])
        );
    }

    #[test]
    fn test_literal() {
        let parts = CssSyntax::new("/").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Literal {
                literal: "/".into(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new(",").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Literal {
                literal: ",".into(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );
    }

    #[test]
    fn test_special_keywords() {
        let parts = CssSyntax::new("unset").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unset {
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("initial").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Initial {
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let parts = CssSyntax::new("unset").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap(),
            CssSyntaxTree::new(vec![SyntaxComponent::Unset {
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );
    }

    #[test]
    fn test_compile_unit() {
        let parts = CssSyntax::new("10px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Value {
                value: CssValue::Unit(10.0, "px".to_string()),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }]
        );

        let parts = CssSyntax::new("10.43px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Value {
                value: CssValue::Unit(10.43, "px".to_string()),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }]
        );

        let parts = CssSyntax::new("-10.43px").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Value {
                value: CssValue::Unit(-10.43, "px".to_string()),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }]
        );

        let parts = CssSyntax::new("0").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Value {
                value: CssValue::Zero,
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }]
        );
    }

    #[test]
    fn test_compile_datatype() {
        let parts = CssSyntax::new("<foo> | <bar()> | <'quoted'>").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Definition {
                        datatype: "foo".to_string(),
                        quoted: false,
                        range: RangeType::empty(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Definition {
                        datatype: "bar()".to_string(),
                        quoted: false,
                        range: RangeType::empty(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Definition {
                        datatype: "quoted".to_string(),
                        quoted: true,
                        range: RangeType::empty(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }]
        );

        let parts = CssSyntax::new("<foo>#").compile();
        assert!(parts.is_ok());
        assert_eq!(
            parts.unwrap().components,
            vec![SyntaxComponent::Definition {
                datatype: "foo".to_string(),
                quoted: false,
                range: RangeType::empty(),
                multipliers: vec![SyntaxComponentMultiplier::CommaSeparatedRepeat(
                    1,
                    u32::MAX as usize
                )],
            }]
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
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left | right && top").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left && right | top").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "top".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left && right || top").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::AtLeastOneAnyOrder,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "top".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left || right | top").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "top".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left | right || top").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left | right || top && bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::AllAnyOrder,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "top".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "bottom".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left || right | top && bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "bottom".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left && right || top | bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::AllAnyOrder,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "left".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "right".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "bottom".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left  right || top | bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::Juxtaposition,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "left".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "right".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "bottom".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left | right || top | bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "bottom".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left || right | top || bottom")
            .compile()
            .unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "bottom".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("left right | top bottom").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".into(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "bottom".into(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );
    }

    #[test]
    fn test_datatype_ranges() {
        let c = CssSyntax::new("<function [1, 2]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::FiniteI64(1),
                    max: NumberOrInfinity::FiniteI64(2),
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("<function>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::None,
                    max: NumberOrInfinity::None,
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("<function [1,]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::FiniteI64(1),
                    max: NumberOrInfinity::None,
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("<function [,1]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::None,
                    max: NumberOrInfinity::FiniteI64(1),
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("<function [-360,360]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::FiniteI64(-360),
                    max: NumberOrInfinity::FiniteI64(360),
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );

        let c = CssSyntax::new("<function [0,inf]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::FiniteI64(0),
                    max: NumberOrInfinity::Infinity,
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );
        let c = CssSyntax::new("<function [-inf, 0]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::NegativeInfinity,
                    max: NumberOrInfinity::FiniteI64(0),
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );
        let c = CssSyntax::new("<function [-inf,inf]>").compile().unwrap();
        assert_eq!(
            c.components[0],
            SyntaxComponent::Definition {
                datatype: "function".to_string(),
                quoted: false,
                range: RangeType {
                    min: NumberOrInfinity::NegativeInfinity,
                    max: NumberOrInfinity::Infinity,
                },
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }
        );
    }

    #[test]
    fn test_specific_precedence_configurations() {
        // @todo: These tests should also check if the syntax is correct, not only if it can
        // compile. The output could still be wrong.
        assert!(CssSyntax::new("le, ri ,co , bt,tp").compile().is_ok());
        assert!(CssSyntax::new("left | right | center && top")
            .compile()
            .is_ok());
        assert!(CssSyntax::new("left , right color()").compile().is_ok());
        assert!(CssSyntax::new("left , right color() ").compile().is_ok());
        assert!(CssSyntax::new("le, ri ,co , bt,tp").compile().is_ok());
        assert!(CssSyntax::new("left, right color()").compile().is_ok());
        assert!(CssSyntax::new("left | right | center && top")
            .compile()
            .is_ok());
        assert!(CssSyntax::new("left | right | center && top <length>")
            .compile()
            .is_ok());
        assert!(CssSyntax::new("[ [ <length-percentage>? ]]")
            .compile()
            .is_ok());
        assert!(CssSyntax::new("[ [ center | [ top | bottom ]  ]]")
            .compile()
            .is_ok());
        assert!(CssSyntax::new("[ <length-percentage>? ]").compile().is_ok());
        assert!(CssSyntax::new("[ center <length-percentage>? ]")
            .compile()
            .is_ok());
        assert!(
            CssSyntax::new("center | [ top | bottom ] <length-percentage>")
                .compile()
                .is_ok()
        );
        assert!(
            CssSyntax::new("[ center | [ top | bottom ] <length-percentage> ]")
                .compile()
                .is_ok()
        );
        assert!(
            CssSyntax::new("[ center | [ top | bottom ] <length-percentage>? ]")
                .compile()
                .is_ok()
        );
        assert!(
            CssSyntax::new("[ [ center | [ top | bottom ] <length-percentage>? ]]")
                .compile()
                .is_ok()
        );
        assert!(CssSyntax::new("[ [ top | center | bottom | <length-percentage> ]| [ center | [ left | right ] <length-percentage>? ] && [ center | [ top | bottom ] <length-percentage>? ]]").compile().is_ok());
        assert!(CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ]| [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ]| [ center | [ left | right ] <length-percentage>? ] && [ center | [ top | bottom ] <length-percentage>? ]]").compile().is_ok());
    }

    #[test]
    fn test_anchor_with_arguments() {
        let c = CssSyntax::new("anchor( <anchor-element>? <anchor-side>, <length-percentage>? )")
            .compile()
            .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Function {
                name: "anchor".to_string(),
                arguments: Some(Box::new(SyntaxComponent::Group {
                    combinator: GroupCombinators::Juxtaposition,
                    components: vec![
                        SyntaxComponent::Definition {
                            datatype: "anchor-element".to_string(),
                            quoted: false,
                            range: RangeType::empty(),
                            multipliers: vec![SyntaxComponentMultiplier::Optional],
                        },
                        SyntaxComponent::Definition {
                            datatype: "anchor-side".to_string(),
                            quoted: false,
                            range: RangeType::empty(),
                            multipliers: vec![SyntaxComponentMultiplier::Once],
                        },
                        SyntaxComponent::Literal {
                            literal: ",".to_string(),
                            multipliers: vec![SyntaxComponentMultiplier::Once],
                        },
                        SyntaxComponent::Definition {
                            datatype: "length-percentage".to_string(),
                            quoted: false,
                            range: RangeType {
                                min: NumberOrInfinity::None,
                                max: NumberOrInfinity::None,
                            },
                            multipliers: vec![SyntaxComponentMultiplier::Optional],
                        },
                    ],
                    multipliers: vec![SyntaxComponentMultiplier::Once],
                })),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );
    }

    #[test]
    fn test_grouping_precedence() {
        let c = CssSyntax::new("left").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "left".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("left right").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("left right | top bottom").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "bottom".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("left && right | foo || bar baz")
            .compile()
            .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AtLeastOneAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "foo".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::Juxtaposition,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "bar".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "baz".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ left ] [ right ] [ top ]")
            .compile()
            .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "top".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ left right top ]").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "top".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ [ left ] ]").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::GenericKeyword {
                keyword: "left".to_string(),
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ left right ]").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[left right]").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ [ left ] ] right ").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "right".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ [ left ] [ right ] [ top ] ] a")
            .compile()
            .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::Juxtaposition,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::GenericKeyword {
                        keyword: "a".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ [ left ] | [ right ] [ top ] ]")
            .compile()
            .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new("[ [ left | center | right | top | bottom | <length-percentage> ] | [ left | center | right | <length-percentage> ] [ top | center | bottom | <length-percentage> ] | [ center | [ left | right ] <length-percentage>? ] && [ center | [ top | bottom ] <length-percentage>? ] ]").compile().unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::ExactlyOne,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "left".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "center".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "right".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "bottom".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::Definition {
                                datatype: "length-percentage".into(),
                                quoted: false,
                                range: RangeType {
                                    min: NumberOrInfinity::None,
                                    max: NumberOrInfinity::None,
                                },
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::ExactlyOne,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "left".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "center".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "right".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::Definition {
                                        datatype: "length-percentage".into(),
                                        quoted: false,
                                        range: RangeType {
                                            min: NumberOrInfinity::None,
                                            max: NumberOrInfinity::None,
                                        },
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::ExactlyOne,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "top".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "center".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "bottom".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::Definition {
                                        datatype: "length-percentage".into(),
                                        quoted: false,
                                        range: RangeType {
                                            min: NumberOrInfinity::None,
                                            max: NumberOrInfinity::None,
                                        },
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::ExactlyOne,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "center".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::Group {
                                        combinator: GroupCombinators::Juxtaposition,
                                        components: vec![
                                            SyntaxComponent::Group {
                                                combinator: GroupCombinators::ExactlyOne,
                                                components: vec![
                                                    SyntaxComponent::GenericKeyword {
                                                        keyword: "left".to_string(),
                                                        multipliers: vec![
                                                            SyntaxComponentMultiplier::Once
                                                        ],
                                                    },
                                                    SyntaxComponent::GenericKeyword {
                                                        keyword: "right".to_string(),
                                                        multipliers: vec![
                                                            SyntaxComponentMultiplier::Once
                                                        ],
                                                    },
                                                ],
                                                multipliers: vec![SyntaxComponentMultiplier::Once],
                                            },
                                            SyntaxComponent::Definition {
                                                datatype: "length-percentage".into(),
                                                quoted: false,
                                                range: RangeType {
                                                    min: NumberOrInfinity::None,
                                                    max: NumberOrInfinity::None,
                                                },
                                                multipliers: vec![
                                                    SyntaxComponentMultiplier::Optional
                                                ],
                                            },
                                        ],
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::Group {
                                combinator: GroupCombinators::ExactlyOne,
                                components: vec![
                                    SyntaxComponent::GenericKeyword {
                                        keyword: "center".to_string(),
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                    SyntaxComponent::Group {
                                        combinator: GroupCombinators::Juxtaposition,
                                        components: vec![
                                            SyntaxComponent::Group {
                                                combinator: GroupCombinators::ExactlyOne,
                                                components: vec![
                                                    SyntaxComponent::GenericKeyword {
                                                        keyword: "top".to_string(),
                                                        multipliers: vec![
                                                            SyntaxComponentMultiplier::Once
                                                        ],
                                                    },
                                                    SyntaxComponent::GenericKeyword {
                                                        keyword: "bottom".to_string(),
                                                        multipliers: vec![
                                                            SyntaxComponentMultiplier::Once
                                                        ],
                                                    },
                                                ],
                                                multipliers: vec![SyntaxComponentMultiplier::Once],
                                            },
                                            SyntaxComponent::Definition {
                                                datatype: "length-percentage".into(),
                                                quoted: false,
                                                range: RangeType {
                                                    min: NumberOrInfinity::None,
                                                    max: NumberOrInfinity::None,
                                                },
                                                multipliers: vec![
                                                    SyntaxComponentMultiplier::Optional
                                                ],
                                            },
                                        ],
                                        multipliers: vec![SyntaxComponentMultiplier::Once],
                                    },
                                ],
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::Once],
            }])
        );

        let c = CssSyntax::new(
            "[ [ left+ ] | [ center? ] [ top# ]{1,3} | [ center1 ]? && [ center2 ] ]*",
        )
        .compile()
        .unwrap();
        assert_eq!(
            c,
            CssSyntaxTree::new(vec![SyntaxComponent::Group {
                combinator: GroupCombinators::ExactlyOne,
                components: vec![
                    SyntaxComponent::GenericKeyword {
                        keyword: "left".to_string(),
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::Juxtaposition,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "center".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "top".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Between(1, 3)],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                    SyntaxComponent::Group {
                        combinator: GroupCombinators::AllAnyOrder,
                        components: vec![
                            SyntaxComponent::GenericKeyword {
                                keyword: "center1".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Optional],
                            },
                            SyntaxComponent::GenericKeyword {
                                keyword: "center2".to_string(),
                                multipliers: vec![SyntaxComponentMultiplier::Once],
                            },
                        ],
                        multipliers: vec![SyntaxComponentMultiplier::Once],
                    },
                ],
                multipliers: vec![SyntaxComponentMultiplier::ZeroOrMore],
            }])
        );
    }
}
