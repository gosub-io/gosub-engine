use core::fmt::Debug;
use std::cmp::Ordering;
use std::fmt::Display;

use anyhow::anyhow;

use gosub_shared::types::Result;

use crate::colors::RgbColor;

/// Defines a complete stylesheet with all its rules and the location where it was found
#[derive(Debug, PartialEq, Clone)]
pub struct CssStylesheet {
    /// List of rules found in this stylesheet
    pub rules: Vec<CssRule>,
    /// Origin of the stylesheet (user agent, author, user)
    pub origin: CssOrigin,
    /// Url or file path where the stylesheet was found
    pub location: String,
}

/// Defines the origin of the stylesheet (or declaration)
#[derive(Debug, PartialEq, Clone)]
pub enum CssOrigin {
    /// Browser/user agent defined stylesheets
    UserAgent,
    /// Author defined stylesheets that are linked or embedded in the HTML files
    Author,
    /// User defined stylesheets that will override the author and user agent stylesheets (for instance, custom user styles or extensions)
    User,
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
    pub fn selectors(&self) -> &Vec<CssSelector> {
        &self.selectors
    }

    pub fn declarations(&self) -> &Vec<CssDeclaration> {
        &self.declarations
    }
}

/// A CSS declaration, which contains a property, value and a flag for !important
#[derive(Debug, PartialEq, Clone)]
pub struct CssDeclaration {
    // Css property color
    pub property: String,
    // Raw values of the declaration. It is not calculated or converted in any way (ie: "red", "50px" etc)
    // There can be multiple values  (ie:   "1px solid black" are split into 3 values)
    pub value: Vec<CssValue>,
    // ie: !important
    pub important: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub struct CssSelector {
    // List of parts that make up this selector
    pub parts: Vec<Vec<CssSelectorPart>>,
}

impl CssSelector {
    /// Generate specificity for this selector
    pub fn specificity(&self) -> Vec<Specificity> {
        self.parts
            .iter()
            .map(|part| Specificity::from(part.as_slice()))
            .collect()
    }
}

/// Represents a CSS selector part, which has a type and value (e.g. type=Class, class="my-class")
#[derive(PartialEq, Clone, Default)]
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

#[derive(PartialEq, Clone, Default, Debug)]
pub struct AttributeSelector {
    pub name: String,
    pub matcher: MatcherType,
    pub value: String,
    pub case_insensitive: bool,
}

#[derive(Debug, PartialEq, Clone)]
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
            Combinator::Descendant => write!(f, " "),
            Combinator::Child => write!(f, ">"),
            Combinator::NextSibling => write!(f, "+"),
            Combinator::SubsequentSibling => write!(f, "~"),
            Combinator::Column => write!(f, "||"),
            Combinator::Namespace => write!(f, "|"),
        }
    }
}

impl Debug for CssSelectorPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssSelectorPart::Universal => {
                write!(f, "*")
            }
            CssSelectorPart::Attribute(selector) => {
                write!(
                    f,
                    "[{} {} {} {}]",
                    selector.name, selector.matcher, selector.value, selector.case_insensitive
                )
            }
            CssSelectorPart::Class(name) => {
                write!(f, ".{}", name)
            }
            CssSelectorPart::Id(name) => {
                write!(f, "#{}", name)
            }
            CssSelectorPart::PseudoClass(name) => {
                write!(f, ":{}", name)
            }
            CssSelectorPart::PseudoElement(name) => {
                write!(f, "::{}", name)
            }
            CssSelectorPart::Combinator(combinator) => {
                write!(f, "'{}'", combinator)
            }
            CssSelectorPart::Type(name) => {
                write!(f, "{}", name)
            }
        }
    }
}

/// Represents a CSS selector type for this part
#[derive(Debug, PartialEq, Clone, Default)]
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
#[derive(Default, PartialEq, Clone, Debug)]
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
            MatcherType::None => write!(f, ""),
            MatcherType::Equals => write!(f, "="),
            MatcherType::Includes => write!(f, "~="),
            MatcherType::DashMatch => write!(f, "|="),
            MatcherType::PrefixMatch => write!(f, "^="),
            MatcherType::SuffixMatch => write!(f, "$="),
            MatcherType::SubstringMatch => write!(f, "*="),
        }
    }
}

/// Defines the specificity for a selector
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Specificity(u32, u32, u32);

impl Specificity {
    pub fn new(a: u32, b: u32, c: u32) -> Self {
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
        Specificity::new(id_count, class_count, element_count)
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
    Function(String, Vec<CssValue>),
    Initial,
    Inherit,
    Comma,
    List(Vec<CssValue>),
}

impl Display for CssValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssValue::None => write!(f, "none"),
            CssValue::Color(col) => {
                write!(
                    f,
                    "#{:02x}{:02x}{:02x}{:02x}",
                    col.r as u8, col.g as u8, col.b as u8, col.a as u8
                )
            }
            CssValue::Zero => write!(f, "0"),
            CssValue::Number(num) => write!(f, "{}", num),
            CssValue::Percentage(p) => write!(f, "{}%", p),
            CssValue::String(s) => write!(f, "{}", s),
            CssValue::Unit(val, unit) => write!(f, "{}{}", val, unit),
            CssValue::Function(name, args) => {
                write!(f, "{}(", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ")")
            }
            CssValue::Initial => write!(f, "initial"),
            CssValue::Inherit => write!(f, "inherit"),
            CssValue::Comma => write!(f, ","),
            CssValue::List(v) => {
                write!(f, "List(")?;
                for (i, value) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", value)?;
                }
                write!(f, ")")
            }
        }
    }
}

impl CssValue {
    pub fn to_color(&self) -> Option<RgbColor> {
        match self {
            CssValue::Color(col) => Some(*col),
            CssValue::String(s) => Some(RgbColor::from(s.as_str())),
            _ => None,
        }
    }

    pub fn unit_to_px(&self) -> f32 {
        //TODO: Implement the rest of the units
        match self {
            CssValue::Unit(val, unit) => match unit.as_str() {
                "px" => *val,
                "em" => *val * 16.0,
                "rem" => *val * 16.0,
                _ => *val,
            },
            CssValue::String(value) => {
                if value.ends_with("px") {
                    value.trim_end_matches("px").parse::<f32>().unwrap()
                } else if value.ends_with("rem") {
                    value.trim_end_matches("rem").parse::<f32>().unwrap() * 16.0
                } else if value.ends_with("em") {
                    value.trim_end_matches("em").parse::<f32>().unwrap() * 16.0
                } else if value.ends_with("__qem") {
                    value.trim_end_matches("__qem").parse::<f32>().unwrap() * 16.0
                } else {
                    0.0
                }
            }
            _ => 0.0,
        }
    }

    /// Converts a CSS AST node to a CSS value
    pub fn parse_ast_node(node: &crate::node::Node) -> Result<CssValue> {
        match *node.node_type.clone() {
            crate::node::NodeType::Ident { value } => Ok(CssValue::String(value)),
            crate::node::NodeType::Number { value } => {
                if value == 0.0 {
                    // Zero is a special case since we need to do some pattern matching once in a while, and
                    // this is not possible (anymore) with floating point 0.0 it seems
                    Ok(CssValue::Zero)
                } else {
                    Ok(CssValue::Number(value))
                }
            }
            crate::node::NodeType::Percentage { value } => Ok(CssValue::Percentage(value)),
            crate::node::NodeType::Dimension { value, unit } => Ok(CssValue::Unit(value, unit)),
            crate::node::NodeType::String { value } => Ok(CssValue::String(value)),
            crate::node::NodeType::Hash { mut value } => {
                value.insert(0, '#');

                Ok(CssValue::String(value))
            }
            crate::node::NodeType::Operator(_) => Ok(CssValue::None),
            crate::node::NodeType::Calc { .. } => {
                Ok(CssValue::Function("calc".to_string(), vec![]))
            }
            crate::node::NodeType::Url { url } => Ok(CssValue::Function(
                "url".to_string(),
                vec![CssValue::String(url)],
            )),
            crate::node::NodeType::Function { name, arguments } => {
                let mut list = vec![];
                for node in arguments.iter() {
                    match CssValue::parse_ast_node(node) {
                        Ok(value) => list.push(value),
                        Err(e) => return Err(e),
                    }
                }
                Ok(CssValue::Function(name, list))
            }
            _ => Err(anyhow!(format!(
                "Cannot convert node to CssValue: {:?}",
                node
            ))),
        }
    }

    /// Parses a string into a CSS value or list of css values
    pub fn parse_str(value: &str) -> Result<CssValue> {
        match value {
            "initial" => return Ok(CssValue::Initial),
            "inherit" => return Ok(CssValue::Inherit),
            "none" => return Ok(CssValue::None),
            "" => return Ok(CssValue::String("".into())),
            _ => {}
        }

        if let Ok(num) = value.parse::<f32>() {
            return Ok(CssValue::Number(num));
        }

        // Color values
        if value.starts_with("color(") && value.ends_with(')') {
            return Ok(CssValue::Color(RgbColor::from(
                value[6..value.len() - 1].to_string().as_str(),
            )));
        }

        // Percentages
        if value.ends_with('%') {
            if let Ok(num) = value[0..value.len() - 1].parse::<f32>() {
                return Ok(CssValue::Percentage(num));
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
                return Ok(CssValue::Unit(number, unit_part.to_string()));
            }
        }

        Ok(CssValue::String(value.to_string()))
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
                value: vec![CssValue::String("red".to_string())],
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
