use crate::css3::node::Node as CssNode;
use anyhow::{anyhow, Result};
use core::fmt::Display;
use std::fmt::Debug;

/*

Given the following css:

    * { color: red; }
    h1 { color: blue; }
    h3, h4 { color: rebeccapurple; }
    ul > li { color: green; }

this will parse into the following structure:

CssStylesheet
    Rule
        SelectorList
            SelectorGroup
                Selector: Universal *
    Rule
        SelectorList
            SelectorGroup
                part: Ident h1
    Rule
        SelectorList
            Selector
                part: Ident h3
            Selector
                part: Ident h4
    Rule
        SelectorList
            Selector
                part: Ident	ul
                part: Combinator	>
                part: Ident	li

In case of h3, h4, the SelectorList contains two entries in the SelectorList, each with a single Selector. But having 2 rules with each one single
selector list entry would have been the same thing:

    Rule
        SelectorList
            Selector
                part: Ident h3
    Rule
        SelectorList
            Selector
                part: Ident h4


in css:

    h3, h4 { color: rebeccapurple; }

vs

    h3 { color: rebeccapurple; }
    h4 { color: rebeccapurple; }

 */

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

#[derive(Default, PartialEq, Clone)]
pub enum MatcherType {
    #[default]
    None,
    Equals,
    Includes,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
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

/// Represents a CSS selector part, which has a type and value (e.g. type=Class, class="my-class")
#[derive(PartialEq, Clone, Default)]
pub struct CssSelectorPart {
    pub(crate) type_: CssSelectorType,
    pub(crate) value: String,
    pub(crate) matcher: MatcherType,
    pub(crate) name: String,
    pub(crate) flags: String,
}

impl Debug for CssSelectorPart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.type_ {
            CssSelectorType::Universal => {
                write!(f, "*")
            }
            CssSelectorType::Attribute => {
                write!(
                    f,
                    "[{} {} {} {}]",
                    self.name, self.matcher, self.value, self.flags
                )
            }
            CssSelectorType::Class => {
                write!(f, ".{}", self.value)
            }
            CssSelectorType::Id => {
                write!(f, "#{}", self.value)
            }
            CssSelectorType::PseudoClass => {
                write!(f, ":{}", self.value)
            }
            CssSelectorType::PseudoElement => {
                write!(f, "::{}", self.value)
            }
            CssSelectorType::Combinator => {
                write!(f, "'{}'", self.value)
            }
            CssSelectorType::Type => {
                write!(f, "{}", self.value)
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Specificity(u32, u32, u32);

impl Specificity {
    pub fn new(a: u32, b: u32, c: u32) -> Self {
        Self(a, b, c)
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.0.cmp(&other.0) {
            std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => match self.1.cmp(&other.1) {
                std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
                std::cmp::Ordering::Less => std::cmp::Ordering::Less,
                std::cmp::Ordering::Equal => match self.2.cmp(&other.2) {
                    std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
                    std::cmp::Ordering::Less => std::cmp::Ordering::Less,
                    std::cmp::Ordering::Equal => std::cmp::Ordering::Equal,
                },
            },
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct CssSelector {
    pub(crate) parts: Vec<CssSelectorPart>,
}

impl CssSelector {
    /// Generate specificity for this selector
    pub fn specificity(&self) -> Specificity {
        let mut id_count = 0;
        let mut class_count = 0;
        let mut element_count = 0;
        for part in &self.parts {
            match part.type_ {
                CssSelectorType::Id => {
                    id_count += 1;
                }
                CssSelectorType::Class => {
                    class_count += 1;
                }
                CssSelectorType::Type => {
                    element_count += 1;
                }
                _ => {}
            }
        }
        Specificity::new(id_count, class_count, element_count)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct CssRule {
    selectors: Vec<CssSelector>,
    declarations: Vec<CssDeclaration>,
}

impl CssRule {
    pub fn selectors(&self) -> &Vec<CssSelector> {
        &self.selectors
    }

    pub fn declarations(&self) -> &Vec<CssDeclaration> {
        &self.declarations
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct CssStylesheet {
    pub rules: Vec<CssRule>,
    pub origin: CssOrigin,
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

#[derive(Debug, PartialEq, Clone)]
pub struct CssDeclaration {
    // ie: color
    pub property: String,
    // ie: red
    pub value: String,
    // ie: !important
    pub important: bool,
}

/// Converts a CSS AST to a CSS stylesheet structure that can be used for easy matching
pub fn convert_css_ast_to_rules(
    css_ast: &CssNode,
    origin: CssOrigin,
    location: &str,
) -> Result<CssStylesheet> {
    if !css_ast.is_stylesheet() {
        return Err(anyhow!("CSS AST must be starting as a stylesheet"));
    }

    let mut sheet = CssStylesheet {
        rules: vec![],
        origin,
        location: location.to_string(),
    };

    for node in css_ast.as_stylesheet() {
        if !node.is_rule() {
            continue;
        }

        let mut rule = CssRule {
            selectors: vec![],
            declarations: vec![],
        };

        let (prelude, declarations) = node.as_rule();
        for node in prelude.iter() {
            if !node.is_selector_list() {
                continue;
            }

            let mut selector = CssSelector { parts: vec![] };

            for node in node.as_selector_list().iter() {
                if !node.is_selector() {
                    continue;
                }

                for node in node.as_selector() {
                    if node.is_ident() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: node.as_ident().clone(),
                            ..Default::default()
                        });
                    } else if node.is_class_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Class,
                            value: node.as_class_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_combinator() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Combinator,
                            value: node.as_combinator().clone(),
                            ..Default::default()
                        });
                    } else if node.is_id_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Id,
                            value: node.as_id_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_universal_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Universal,
                            value: "*".to_string(),
                            ..Default::default()
                        });
                    } else if node.is_pseudo_class_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::PseudoClass,
                            value: node.as_pseudo_class_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_pseudo_element_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::PseudoElement,
                            value: node.as_pseudo_element_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_type_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: node.as_type_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_attribute_selector() {
                        let attr_selector = node.as_attribute_selector();
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Attribute,
                            name: attr_selector.0.clone(),
                            matcher: MatcherType::Equals, // @todo: this needs to be parsed
                            value: attr_selector.2.clone(),
                            flags: attr_selector.3.clone(),
                        });
                    } else {
                        panic!("Unknown selector type: {:?}", node);
                    }
                }
            }
            rule.selectors.push(selector);
        }

        for declaration in declarations.iter() {
            if !declaration.is_block() {
                continue;
            }

            let block = declaration.as_block();
            for declaration in block.iter() {
                if !declaration.is_declaration() {
                    continue;
                }

                let (property, value, important) = declaration.as_declaration();
                rule.declarations.push(CssDeclaration {
                    property: property.clone(),
                    value: value[0].to_string(),
                    important: *important,
                });
            }
        }

        sheet.rules.push(rule);
    }
    Ok(sheet)
}
