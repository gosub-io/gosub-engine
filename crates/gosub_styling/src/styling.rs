use core::fmt::Debug;
use std::cmp::Ordering;
use std::collections::HashMap;

use crate::css_definitions::get_css_definitions;
use gosub_css3::stylesheet::{
    CssOrigin, CssSelector, CssSelectorPart, CssSelectorType, CssValue, MatcherType, Specificity,
};
use gosub_html5::node::NodeId;
use gosub_html5::parser::document::DocumentHandle;

// Matches a complete selector (all parts) against the given node(id)
pub(crate) fn match_selector(
    document: DocumentHandle,
    node_id: NodeId,
    selector: &CssSelector,
) -> bool {
    let mut parts = selector.parts.clone();
    parts.reverse();
    match_selector_part(document, node_id, &mut parts)
}

/// Returns true when the given node matches the part(s)
fn match_selector_part(
    document: DocumentHandle,
    node_id: NodeId,
    selector_parts: &mut Vec<CssSelectorPart>,
) -> bool {
    let binding = document.get();
    let mut next_current_node = Some(binding.get_node_by_id(node_id).expect("node not found"));

    while !selector_parts.is_empty() {
        if next_current_node.is_none() {
            return false;
        }
        let current_node = next_current_node.expect("current_node not found");
        if current_node.is_root() {
            return false;
        }

        let part = selector_parts.remove(0);

        match part.type_ {
            CssSelectorType::Universal => {
                // '*' always matches any selector
            }
            CssSelectorType::Type => {
                if !current_node.is_element() {
                    return false;
                }
                if part.value != current_node.as_element().name {
                    return false;
                }
            }
            CssSelectorType::Class => {
                if !current_node.is_element() {
                    return false;
                }
                if !current_node.as_element().classes.contains(&part.value) {
                    return false;
                }
            }
            CssSelectorType::Id => {
                if !current_node.is_element() {
                    return false;
                }
                if current_node
                    .as_element()
                    .attributes
                    .get("id")
                    .unwrap_or(&"".to_string())
                    != &part.value
                {
                    return false;
                }
            }
            CssSelectorType::Attribute => {
                let wanted_attr_name = part.name.clone();

                if !current_node.has_attribute(&wanted_attr_name) {
                    return false;
                }

                let mut wanted_attr_value = part.value.clone();
                let mut got_attr_value = current_node
                    .get_attribute(&wanted_attr_name)
                    .unwrap_or(&"".to_string())
                    .to_string();

                // If we need to match case-insensitive, just convert everything to lowercase for comparison
                if part.flags.eq_ignore_ascii_case("i") {
                    wanted_attr_value = wanted_attr_value.to_lowercase();
                    got_attr_value = got_attr_value.to_lowercase();
                };

                return match part.matcher {
                    MatcherType::None => {
                        // Just the presence of the attribute is enough
                        true
                    }
                    MatcherType::Equals => {
                        // Exact match
                        wanted_attr_value == got_attr_value
                    }
                    MatcherType::Includes => {
                        // Contains word
                        wanted_attr_value
                            .split_whitespace()
                            .any(|s| s == got_attr_value)
                    }
                    MatcherType::DashMatch => {
                        // Exact value or value followed by a hyphen
                        got_attr_value == wanted_attr_value
                            || got_attr_value.starts_with(&format!("{}-", wanted_attr_value))
                    }
                    MatcherType::PrefixMatch => {
                        // Starts with
                        got_attr_value.starts_with(&wanted_attr_value)
                    }
                    MatcherType::SuffixMatch => {
                        // Ends with
                        got_attr_value.ends_with(&wanted_attr_value)
                    }
                    MatcherType::SubstringMatch => {
                        // Contains
                        got_attr_value.contains(&wanted_attr_value)
                    }
                };
            }
            CssSelectorType::PseudoClass => {
                // @Todo: implement pseudo classes
                if part.value == "link" {
                    return false;
                }
                return false;
            }
            CssSelectorType::PseudoElement => {
                // @Todo: implement pseudo elements
                if part.value == "first-child" {
                    return false;
                }
                return false;
            }
            CssSelectorType::Combinator => {
                // We don't have the descendant combinator (space), as this is the default behaviour
                match part.value.as_str() {
                    // @todo: We also should do: column combinator ('||' experimental)
                    // @todo: Namespace combinator ('|')
                    " " => {
                        // Descendant combinator, any parent that matches the previous selector will do
                        if !match_selector_part(
                            DocumentHandle::clone(&document),
                            current_node.id,
                            selector_parts,
                        ) {
                            // we insert the combinator back so we the next loop will match against the parent node
                            selector_parts.insert(0, part);
                        }
                    }
                    ">" => {
                        // Child combinator. Only matches the direct child
                        if !match_selector_part(
                            DocumentHandle::clone(&document),
                            current_node.id,
                            selector_parts,
                        ) {
                            return false;
                        }
                    }
                    "+" => {
                        // We need to match the previous sibling of the current node
                    }
                    "~" => {
                        // We need to match the previous siblings of the current node
                    }
                    _ => {
                        panic!("Unknown combinator: {}", part.value);
                    }
                }
            }
        }

        // We have matched this part, so we move up the chain
        // let binding = document.get();
        next_current_node = binding.parent_node(current_node);
    }

    // All parts of the selector have matched
    true
}

/// A declarationProperty defines a single value for a property (color: red;). It consists of the value,
/// origin, importance, location and specificity of the declaration.
#[derive(Debug, Clone)]
pub struct DeclarationProperty {
    /// The actual value of the property (@todo: should this be a vec? or do we need to (re-)implement CssValue::List?)
    pub value: CssValue,
    /// Origin of the declaration (user stylesheet, author stylesheet etc.)
    pub origin: CssOrigin,
    /// Whether the declaration is !important
    pub important: bool,
    /// The location of the declaration in the stylesheet (name.css:123) or empty
    pub location: String,
    /// The specificity of the selector that declared this property
    pub specificity: Specificity,
}

impl DeclarationProperty {
    /// Priority of the declaration based on the origin and importance as defined in https://developer.mozilla.org/en-US/docs/Web/CSS/Cascade
    fn priority(&self) -> u8 {
        match self.origin {
            CssOrigin::UserAgent => {
                if self.important {
                    7
                } else {
                    1
                }
            }
            CssOrigin::User => {
                if self.important {
                    6
                } else {
                    2
                }
            }
            CssOrigin::Author => {
                if self.important {
                    5
                } else {
                    3
                }
            }
        }
    }
}

impl PartialEq<Self> for DeclarationProperty {
    fn eq(&self, other: &Self) -> bool {
        self.priority() == other.priority()
    }
}

impl PartialOrd<Self> for DeclarationProperty {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for DeclarationProperty {}

impl Ord for DeclarationProperty {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority().cmp(&other.priority())
    }
}

/// A value entry contains all values for a single property for a single node. It contains the declared values, and
/// all the computed values.
#[derive(Debug, Clone)]
pub struct CssProperty {
    /// The name of the property
    pub name: String,
    /// True when this property needs to be recalculated
    pub dirty: bool,
    /// List of all declared values for this property
    pub declared: Vec<DeclarationProperty>,
    /// Cascaded value from the declared values (if any)
    pub cascaded: Option<CssValue>,
    // Specified value from the cascaded value (if any), or inherited value, or initial value
    pub specified: CssValue,
    // Computed value from the specified value (needs viewport size etc.)
    pub computed: CssValue,
    pub used: CssValue,
    // Actual value used in the rendering (after rounding, clipping etc.)
    pub actual: CssValue,
}

impl CssProperty {
    pub fn new(prop_name: &str) -> Self {
        Self {
            name: prop_name.to_string(),
            dirty: true,
            declared: Vec::new(),
            cascaded: None,
            specified: CssValue::None,
            computed: CssValue::None,
            used: CssValue::None,
            actual: CssValue::None,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Returns the actual value of the property. Will compute the value when needed
    pub fn compute_value(&mut self) -> &CssValue {
        if self.dirty {
            self.calculate_value();
            self.dirty = false;
        }

        &self.actual
    }

    fn calculate_value(&mut self) {
        self.cascaded = self.find_cascaded_value();
        self.specified = self.find_specified_value();
        self.computed = self.find_computed_value();
        self.used = self.find_used_value();
        self.actual = self.find_actual_value();
    }

    fn find_cascaded_value(&self) -> Option<CssValue> {
        let mut declared = self.declared.clone();

        declared.sort();
        declared.sort_by(|a, b| {
            if a.priority() == b.priority() {
                return Ordering::Equal;
            }

            a.specificity.cmp(&b.specificity)
        });

        declared.last().map(|d| d.value.clone())
    }

    fn find_specified_value(&self) -> CssValue {
        match self.declared.iter().max() {
            Some(decl) => decl.value.clone(),
            None => CssValue::None,
        }
    }

    fn find_computed_value(&self) -> CssValue {
        if self.specified != CssValue::None {
            return self.specified.clone();
        }

        if self.is_inheritable() {
            todo!("inheritable properties")
            // while let Some(parent) = self.get_parent() {
            //     if let Some(parent_value) = parent {
            //         return parent_value.find_computed_value();
            //     }
            // }
        }

        self.get_initial_value().unwrap_or(CssValue::None)
    }

    fn find_used_value(&self) -> CssValue {
        self.computed.clone()
    }

    fn find_actual_value(&self) -> CssValue {
        // @TODO: stuff like clipping and such should occur as well
        match &self.used {
            CssValue::Number(len) => CssValue::Number(len.round()),
            CssValue::Percentage(perc) => CssValue::Percentage(perc.round()),
            CssValue::Unit(value, unit) => CssValue::Unit(value.round(), unit.clone()),
            _ => self.used.clone(),
        }
    }

    // // Returns true when the property is inheritable, false otherwise
    fn is_inheritable(&self) -> bool {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => def.inherited(),
            None => false,
        }
    }

    // /// Returns true if the given property is a shorthand property (ie: border, margin etc)
    pub fn is_shorthand(&self) -> bool {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => def.expanded_properties().len() > 1,
            None => false,
        }
    }

    /// Returns the list of properties from a shorthand property, or just the property itself if it isn't a shorthand property.
    pub fn get_props_from_shorthand(&self) -> Vec<String> {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => {
                let props = def.expanded_properties();
                if props.len() == 1 {
                    vec![]
                } else {
                    props
                }
            }
            None => vec![],
        }
    }

    // // Returns the initial value for the property, if any
    fn get_initial_value(&self) -> Option<CssValue> {
        let defs = get_css_definitions();
        defs.find_property(&self.name)
            .map(|def| def.initial_value())
    }
}

/// Map of all declared values for a single node. Note that these are only the defined properties, not
/// the non-existing properties.
#[derive(Debug)]
pub struct CssProperties {
    pub properties: HashMap<String, CssProperty>,
}

impl Default for CssProperties {
    fn default() -> Self {
        Self::new()
    }
}

impl CssProperties {
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
        }
    }

    pub fn get(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.properties.get_mut(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_css3::colors::RgbColor;

    #[test]
    fn css_props() {
        let mut props = CssProperties::new();
        let prop = CssProperty::new("color");
        props.properties.insert("color".into(), prop);

        let prop = props.get("color").unwrap();
        assert_eq!(prop.name, "color");

        let prop = props.get("not-exists");
        assert!(prop.is_none());
    }

    #[test]
    fn border_prop_test() {
        let mut prop = CssProperty::new("border");

        prop.declared.push(DeclarationProperty {
            value: CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color(RgbColor::new(255.0, 0.0, 0.0, 255.0)),
            ]),
            origin: CssOrigin::Author,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        });

        assert_eq!(
            prop.compute_value(),
            &CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::Color("red".into()),
            ])
        );
        assert!(prop.is_shorthand());
        assert_eq!(prop.name, "border");
        assert_eq!(prop.get_initial_value(), Some(CssValue::None));
        assert!(!prop.is_inheritable());
    }

    #[test]
    fn color_prop_test() {
        let mut prop = CssProperty::new("color");

        prop.declared.push(DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        });

        assert_eq!(prop.compute_value(), &CssValue::String("red".into()));
        assert!(!prop.is_shorthand());
        assert_eq!(prop.name, "color");
        assert_eq!(prop.get_initial_value(), Some(&CssValue::None).cloned());
        assert!(prop.is_inheritable());
    }

    #[test]
    fn compare_declared() {
        let a = DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let b = DeclarationProperty {
            value: CssValue::String("blue".into()),
            origin: CssOrigin::UserAgent,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let c = DeclarationProperty {
            value: CssValue::String("green".into()),
            origin: CssOrigin::User,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let d = DeclarationProperty {
            value: CssValue::String("yellow".into()),
            origin: CssOrigin::Author,
            important: true,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let e = DeclarationProperty {
            value: CssValue::String("orange".into()),
            origin: CssOrigin::UserAgent,
            important: true,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let f = DeclarationProperty {
            value: CssValue::String("purple".into()),
            origin: CssOrigin::User,
            important: true,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };

        assert_eq!(3, a.priority());
        assert_eq!(1, b.priority());
        assert_eq!(2, c.priority());
        assert_eq!(5, d.priority());
        assert_eq!(7, e.priority());
        assert_eq!(6, f.priority());

        assert!(a > b);
        assert!(b < c);
        assert!(c < d);
        assert!(d < e);
        assert!(f < e);
        assert!(a < e);
        assert!(b < d);
        assert!(a < d);
        assert!(b < d);
        assert!(c < d);
        assert_eq!(c, c);
        assert_eq!(d, d);
    }

    #[test]
    fn is_inheritable() {
        let prop = CssProperty::new("border");
        assert!(!prop.is_inheritable());

        let prop = CssProperty::new("color");
        assert!(prop.is_inheritable());

        let prop = CssProperty::new("font");
        assert!(prop.is_inheritable());

        let prop = CssProperty::new("border-top-color");
        assert!(!prop.is_inheritable());
    }

    #[test]
    fn shorthand_props() {
        let prop = CssProperty::new("border");
        assert!(prop.is_shorthand());
        assert_eq!(
            prop.get_props_from_shorthand(),
            vec!["border-width", "border-style", "border-color"]
        );
        let prop = CssProperty::new("window");
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());

        let prop = CssProperty::new("border-color");
        assert!(prop.is_shorthand());
        assert_eq!(
            prop.get_props_from_shorthand(),
            vec![
                "border-bottom-color",
                "border-left-color",
                "border-right-color",
                "border-top-color",
            ]
        );

        let prop = CssProperty::new("border-top-color");
        assert!(!prop.is_shorthand());
        assert!(prop.get_props_from_shorthand().is_empty());
    }
}
