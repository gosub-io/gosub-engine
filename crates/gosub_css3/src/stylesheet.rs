use core::fmt::Debug;
use core::slice;
use gosub_interface::css3::CssOrigin;
use gosub_shared::byte_stream::Location;
use gosub_shared::errors::CssError;
use gosub_shared::errors::CssResult;
use std::cmp::Ordering;
use std::fmt::Display;

use crate::colors::RgbColor;

/// Severity of a CSS error
#[derive(Debug, PartialEq, Eq)]
pub enum Severity {
    /// A critical error that will prevent the stylesheet from being applied
    Error,
    /// A warning that will be displayed but will not prevent the stylesheet from being applied
    Warning,
    /// An information message that can be displayed to the user
    Info,
}

impl Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "Error"),
            Self::Warning => write!(f, "Warning"),
            Self::Info => write!(f, "Info"),
        }
    }
}

/// Defines a CSS log during
#[derive(PartialEq, Eq)]
pub struct CssLog {
    /// Severity of the error
    pub severity: Severity,
    /// Error message
    pub message: String,
    /// Location of the error
    pub location: Location,
}

impl CssLog {
    #[must_use]
    pub fn log(severity: Severity, message: &str, location: Location) -> Self {
        Self {
            severity,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn error(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Error,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn warn(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.to_string(),
            location,
        }
    }

    #[must_use]
    pub fn info(message: &str, location: Location) -> Self {
        Self {
            severity: Severity::Info,
            message: message.to_string(),
            location,
        }
    }
}

impl Display for CssLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] ({}:{}): {}",
            self.severity, self.location.line, self.location.column, self.message
        )
    }
}

impl Debug for CssLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] ({}:{}): {}",
            self.severity, self.location.line, self.location.column, self.message
        )
    }
}

/// Defines a complete stylesheet with all its rules and the location where it was found
#[derive(Debug, PartialEq)]
pub struct CssStylesheet {
    /// List of rules found in this stylesheet
    pub rules: Vec<CssRule>,
    /// Origin of the stylesheet (user agent, author, user)
    pub origin: CssOrigin,
    /// Url or file path where the stylesheet was found
    pub url: String,
    /// Any issues during parsing of the stylesheet
    pub parse_log: Vec<CssLog>,
}

impl gosub_interface::css3::CssStylesheet for CssStylesheet {
    fn origin(&self) -> CssOrigin {
        self.origin
    }

    fn url(&self) -> &str {
        &self.url
    }
}

/// A CSS rule, which contains a list of selectors and a list of declarations
#[derive(Debug, PartialEq, Clone)]
pub struct CssRule {
    /// Selectors that must match for the declarations to apply
    pub selectors: Vec<CssSelector>,
    /// Actual declarations that will be applied if the selectors match
    pub declarations: Vec<CssDeclaration>,
}

impl CssRule {
    #[must_use]
    pub const fn selectors(&self) -> &Vec<CssSelector> {
        &self.selectors
    }

    #[must_use]
    pub const fn declarations(&self) -> &Vec<CssDeclaration> {
        &self.declarations
    }
}

/// A CSS declaration, which contains a property, value and a flag for !important
#[derive(Debug, PartialEq, Clone)]
pub struct CssDeclaration {
    // Css property color
    pub property: String,
    // Raw values of the declaration. It is not calculated or converted in any way (ie: "red", "50px" etc.)
    // There can be multiple values  (ie:   "1px solid black" are split into 3 values)
    pub value: CssValue,
    // ie: !important
    pub important: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CssSelector {
    // List of parts that make up this selector
    pub parts: Vec<Vec<CssSelectorPart>>,
}

impl CssSelector {
    /// Generate specificity for this selector
    #[must_use]
    pub fn specificity(&self) -> Vec<Specificity> {
        self.parts
            .iter()
            .map(|part| Specificity::from(part.as_slice()))
            .collect()
    }
}

/// Represents a CSS selector part, which has a type and value (e.g. type=Class, class="my-class")
#[derive(PartialEq, Eq, Clone, Default)]
pub enum CssSelectorPart {
    #[default]
    Universal,
    Attribute(Box<AttributeSelector>),
    Class(String),
    Id(String),
    PseudoClass(String),
    PseudoElement(String),
    Combinator(Combinator),
    Type(String),
}

#[derive(PartialEq, Eq, Clone, Default, Debug)]
pub struct AttributeSelector {
    pub name: String,
    pub matcher: MatcherType,
    pub value: String,
    pub case_insensitive: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
    Column,
    Namespace,
}

impl Display for Combinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Descendant => write!(f, " "),
            Self::Child => write!(f, ">"),
            Self::NextSibling => write!(f, "+"),
            Self::SubsequentSibling => write!(f, "~"),
            Self::Column => write!(f, "||"),
            Self::Namespace => write!(f, "|"),
        }
    }
}

impl Debug for CssSelectorPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Universal => {
                write!(f, "*")
            }
            Self::Attribute(selector) => {
                write!(
                    f,
                    "[{} {} {} {}]",
                    selector.name, selector.matcher, selector.value, selector.case_insensitive
                )
            }
            Self::Class(name) => {
                write!(f, ".{name}")
            }
            Self::Id(name) => {
                write!(f, "#{name}")
            }
            Self::PseudoClass(name) => {
                write!(f, ":{name}")
            }
            Self::PseudoElement(name) => {
                write!(f, "::{name}")
            }
            Self::Combinator(combinator) => {
                write!(f, "'{combinator}'")
            }
            Self::Type(name) => {
                write!(f, "{name}")
            }
        }
    }
}

/// Represents a CSS selector type for this part
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub enum CssSelectorType {
    Universal, // '*'
    #[default]
    Type, //  ul, a, h1, etc
    Attribute, // [type ~= "text" i]  (name, matcher, value, flags)
    Class,     // .myclass
    Id,        // #myid
    PseudoClass, // :hover, :active
    PseudoElement, // ::first-child
    Combinator,
}

/// Represents which type of matcher is used (in case of an attribute selector type)
#[derive(Default, PartialEq, Eq, Clone, Debug)]
pub enum MatcherType {
    #[default]
    None, // No matcher
    Equals,         // Equals
    Includes,       // Must include
    DashMatch,      // Must start with
    PrefixMatch,    // Must begin with
    SuffixMatch,    // Must ends with
    SubstringMatch, // Must contain
}

impl Display for MatcherType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, ""),
            Self::Equals => write!(f, "="),
            Self::Includes => write!(f, "~="),
            Self::DashMatch => write!(f, "|="),
            Self::PrefixMatch => write!(f, "^="),
            Self::SuffixMatch => write!(f, "$="),
            Self::SubstringMatch => write!(f, "*="),
        }
    }
}

/// Defines the specificity for a selector
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Specificity(u32, u32, u32);

impl Specificity {
    #[must_use]
    pub const fn new(a: u32, b: u32, c: u32) -> Self {
        Self(a, b, c)
    }
}

impl From<&[CssSelectorPart]> for Specificity {
    fn from(parts: &[CssSelectorPart]) -> Self {
        let mut id_count = 0;
        let mut class_count = 0;
        let mut element_count = 0;
        for part in parts {
            match part {
                CssSelectorPart::Id(_) => {
                    id_count += 1;
                }
                CssSelectorPart::Class(_) => {
                    class_count += 1;
                }
                CssSelectorPart::Type(_) => {
                    element_count += 1;
                }
                _ => {}
            }
        }
        Self::new(id_count, class_count, element_count)
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.0.cmp(&other.0) {
            Ordering::Greater => Ordering::Greater,
            Ordering::Less => Ordering::Less,
            Ordering::Equal => match self.1.cmp(&other.1) {
                Ordering::Greater => Ordering::Greater,
                Ordering::Less => Ordering::Less,
                Ordering::Equal => match self.2.cmp(&other.2) {
                    Ordering::Greater => Ordering::Greater,
                    Ordering::Less => Ordering::Less,
                    Ordering::Equal => Ordering::Equal,
                },
            },
        }
    }
}

/// Actual CSS value, can be a color, length, percentage, string or unit. Some relative values will be computed
/// from other values (ie: Percent(50) will convert to Length(100) when the parent width is 200)
#[derive(Debug, Clone, PartialEq)]
pub enum CssValue {
    None,
    Color(RgbColor),
    Zero,
    Number(f32),
    Percentage(f32),
    String(String),
    Unit(f32, String),
    Function(String, Vec<Self>),
    Initial,
    Inherit,
    Comma,
    List(Vec<Self>),
}

impl Display for CssValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Color(col) => {
                write!(
                    f,
                    "#{:02x}{:02x}{:02x}{:02x}",
                    col.r as u8, col.g as u8, col.b as u8, col.a as u8
                )
            }
            Self::Zero => write!(f, "0"),
            Self::Number(num) => write!(f, "{num}"),
            Self::Percentage(p) => write!(f, "{p}%"),
            Self::String(s) => write!(f, "{s}"),
            Self::Unit(val, unit) => write!(f, "{val}{unit}"),
            Self::Function(name, args) => {
                write!(f, "{name}(")?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{arg}")?;
                }
                write!(f, ")")
            }
            Self::Initial => write!(f, "initial"),
            Self::Inherit => write!(f, "inherit"),
            Self::Comma => write!(f, ","),
            Self::List(v) => {
                write!(f, "List(")?;
                for (i, value) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{value}")?;
                }
                write!(f, ")")
            }
        }
    }
}

impl CssValue {
    #[must_use]
    pub fn to_color(&self) -> Option<RgbColor> {
        match self {
            Self::Color(col) => Some(*col),
            Self::String(s) => Some(RgbColor::from(s.as_str())),
            _ => None,
        }
    }

    /// # Panics
    ///
    /// Panics if the string value cannot be parsed as a float (e.g., malformed numeric content).
    #[must_use]
    pub fn unit_to_px(&self) -> f32 {
        //TODO: Implement the rest of the units
        match self {
            Self::Unit(val, unit) => match unit.as_str() {
                "em" | "rem" => *val * 16.0,
                _ => *val,
            },
            Self::String(value) => {
                if value.ends_with("px") {
                    value
                        .trim_end_matches("px")
                        .parse::<f32>()
                        .expect("failed to parse px value")
                } else if value.ends_with("rem") {
                    value
                        .trim_end_matches("rem")
                        .parse::<f32>()
                        .expect("failed to parse rem value")
                        * 16.0
                } else if value.ends_with("em") {
                    value
                        .trim_end_matches("em")
                        .parse::<f32>()
                        .expect("failed to parse em value")
                        * 16.0
                } else if value.ends_with("__qem") {
                    value
                        .trim_end_matches("__qem")
                        .parse::<f32>()
                        .expect("failed to parse __qem value")
                        * 16.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    #[must_use]
    pub fn from_vec(mut value: Vec<Self>) -> Self {
        match value.len() {
            0 => Self::None,
            1 => value.swap_remove(0),
            _ => Self::List(value),
        }
    }

    #[must_use]
    pub fn to_slice(&self) -> &[Self] {
        match self {
            Self::List(l) => l,
            this => slice::from_ref(this),
        }
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<Self> {
        match self {
            Self::List(l) => l,
            this => vec![this],
        }
    }

    /// Converts a CSS AST node to a CSS value
    pub fn parse_ast_node(node: &crate::node::Node) -> CssResult<Self> {
        match *node.node_type.clone() {
            crate::node::NodeType::Ident { value } | crate::node::NodeType::String { value } => Ok(Self::String(value)),
            crate::node::NodeType::Number { value } => {
                if value == 0.0 {
                    // Zero is a special case since we need to do some pattern matching once in a while, and
                    // this is not possible (anymore) with floating point 0.0 it seems
                    Ok(Self::Zero)
                } else {
                    Ok(Self::Number(value))
                }
            }
            crate::node::NodeType::Percentage { value } => Ok(Self::Percentage(value)),
            crate::node::NodeType::Dimension { value, unit } => Ok(Self::Unit(value, unit)),
            crate::node::NodeType::Hash { mut value } => {
                value.insert(0, '#');

                Ok(Self::String(value))
            }
            crate::node::NodeType::Operator(_) => Ok(Self::None),
            crate::node::NodeType::Calc { .. } => Ok(Self::Function("calc".to_string(), vec![])),
            crate::node::NodeType::Url { url } => Ok(Self::Function("url".to_string(), vec![Self::String(url)])),
            crate::node::NodeType::Function { name, arguments } => {
                let mut list = vec![];
                for node in &arguments {
                    match Self::parse_ast_node(node) {
                        Ok(value) => list.push(value),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Self::Function(name, list))
            }

            crate::node::NodeType::Comma => Ok(Self::Comma),

            _ => Err(CssError::new(
                format!("Cannot convert node to CssValue: {node:?}").as_str(),
            )),
        }
    }

    /// Parses a string into a CSS value or list of css values
    pub fn parse_str(value: &str) -> CssResult<Self> {
        match value {
            "initial" => return Ok(Self::Initial),
            "inherit" => return Ok(Self::Inherit),
            "none" => return Ok(Self::None),
            "" => return Ok(Self::String(String::new())),
            _ => {}
        }

        if let Ok(num) = value.parse::<f32>() {
            return Ok(Self::Number(num));
        }

        // Color values
        if value.starts_with("color(") && value.ends_with(')') {
            return Ok(Self::Color(RgbColor::from(
                value[6..value.len() - 1].to_string().as_str(),
            )));
        }

        // Percentages
        if value.ends_with('%') {
            if let Ok(num) = value[0..value.len() - 1].parse::<f32>() {
                return Ok(Self::Percentage(num));
            }
        }

        // units. If the value starts with a number and ends with some non-numerical
        let mut split_index = None;
        for (index, char) in value.chars().enumerate() {
            if char.is_alphabetic() {
                split_index = Some(index);
                break;
            }
        }
        if let Some(index) = split_index {
            let (number_part, unit_part) = value.split_at(index);
            if let Ok(number) = number_part.parse::<f32>() {
                return Ok(Self::Unit(number, unit_part.to_string()));
            }
        }

        Ok(Self::String(value.to_string()))
    }
}

impl gosub_interface::css3::CssValue for CssValue {
    fn new_string(value: &str) -> Self {
        Self::String(value.to_string())
    }

    fn new_percentage(value: f32) -> Self {
        Self::Percentage(value)
    }

    fn new_unit(value: f32, unit: String) -> Self {
        Self::Unit(value, unit)
    }

    fn new_color(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::Color(RgbColor::new(r, g, b, a))
    }

    fn new_number(value: f32) -> Self {
        Self::Number(value)
    }

    fn new_list(value: Vec<Self>) -> Self {
        Self::List(value)
    }

    fn unit_to_px(&self) -> f32 {
        self.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let Self::String(str) = &self {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let Self::Percentage(percent) = &self {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let Self::Unit(value, unit) = &self {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let Self::Color(color) = &self {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn as_number(&self) -> Option<f32> {
        if let Self::Number(num) = &self {
            Some(*num)
        } else {
            None
        }
    }

    fn as_list(&self) -> Option<&[Self]> {
        if let Self::List(list) = &self {
            Some(list)
        } else {
            None
        }
    }

    fn as_function(&self) -> Option<(&str, &[Self])> {
        if let Self::Function(name, args) = &self {
            Some((name.as_str(), args))
        } else {
            None
        }
    }

    fn is_comma(&self) -> bool {
        matches!(self, Self::Comma)
    }

    fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

#[cfg(test)]
mod test {
    use std::vec;

    use super::*;

    // #[test]
    // fn test_css_value_to_color() {
    //     assert_eq!(CssValue::from_str("color(#ff0000)").unwrap().to_color().unwrap(), RgbColor::from("#ff0000"));
    //     assert_eq!(CssValue::from_str("'Hello'").unwrap().to_color().unwrap(), RgbColor::from("#000000"));
    // }
    //
    // #[test]
    // fn test_css_value_unit_to_px() {
    //     assert_eq!(CssValue::from_str("10px").unwrap().unit_to_px(), 10.0);
    //     assert_eq!(CssValue::from_str("10em").unwrap().unit_to_px(), 160.0);
    //     assert_eq!(CssValue::from_str("10rem").unwrap().unit_to_px(), 160.0);
    //     assert_eq!(CssValue::from_str("10").unwrap().unit_to_px(), 0.0);
    // }

    #[test]
    fn test_css_rule() {
        let rule = CssRule {
            selectors: vec![CssSelector {
                parts: vec![vec![CssSelectorPart::Type("h1".to_string())]],
            }],
            declarations: vec![CssDeclaration {
                property: "color".to_string(),
                value: CssValue::String("red".to_string()),
                important: false,
            }],
        };

        assert_eq!(rule.selectors().len(), 1);
        let part = rule
            .selectors()
            .first()
            .unwrap()
            .parts
            .first()
            .unwrap()
            .first()
            .unwrap();

        assert_eq!(part, &CssSelectorPart::Type("h1".to_string()));
        assert_eq!(rule.declarations().len(), 1);
        assert_eq!(rule.declarations().first().unwrap().property, "color");
    }

    #[test]
    fn test_specificity() {
        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Type("h1".to_string()),
                CssSelectorPart::Class("myclass".to_string()),
                CssSelectorPart::Id("myid".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(1, 1, 1)]);

        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Type("h1".to_string()),
                CssSelectorPart::Class("myclass".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 1, 1)]);

        let selector = CssSelector {
            parts: vec![vec![CssSelectorPart::Type("h1".to_string())]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 0, 1)]);

        let selector = CssSelector {
            parts: vec![vec![
                CssSelectorPart::Class("myclass".to_string()),
                CssSelectorPart::Class("otherclass".to_string()),
            ]],
        };

        let specificity = selector.specificity();
        assert_eq!(specificity, vec![Specificity::new(0, 2, 0)]);
    }

    #[test]
    fn test_specificity_ordering() {
        let specificity1 = Specificity::new(1, 1, 1);
        let specificity2 = Specificity::new(0, 1, 1);
        let specificity3 = Specificity::new(0, 0, 1);
        let specificity4 = Specificity::new(0, 2, 0);
        let specificity5 = Specificity::new(1, 0, 0);
        let specificity6 = Specificity::new(1, 2, 1);
        let specificity7 = Specificity::new(1, 1, 2);
        let specificity8 = Specificity::new(2, 1, 1);

        assert!(specificity1 > specificity2);
        assert!(specificity2 > specificity3);
        assert!(specificity3 < specificity4);
        assert!(specificity4 < specificity5);
        assert!(specificity5 < specificity6);
        assert!(specificity6 > specificity7);
        assert!(specificity7 < specificity8);
    }
}
