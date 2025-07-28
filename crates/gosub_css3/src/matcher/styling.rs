use core::fmt::Debug;
use cow_utils::CowUtils;
use itertools::Itertools;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Display;

use gosub_interface::config::HasDocument;
use gosub_interface::css3;
use gosub_interface::css3::{CssOrigin, CssPropertyMap};
use gosub_interface::document::Document;

use gosub_interface::node::ClassList;
use gosub_interface::node::ElementDataType;
use gosub_interface::node::Node;
use gosub_shared::node::NodeId;

use crate::matcher::property_definitions::get_css_definitions;
use crate::stylesheet::{Combinator, CssSelector, CssSelectorPart, CssValue, MatcherType, Specificity};
use crate::system::Css3System;

// Matches a complete selector (all parts) against the given node(id)
pub(crate) fn match_selector<C: HasDocument>(
    document: &C::Document,
    node_id: NodeId,
    selector: &CssSelector,
) -> (bool, Specificity) {
    for part in &selector.parts {
        if match_selector_parts::<C>(document, node_id, part) {
            return (true, Specificity::from(part.as_slice()));
        }
    }

    (false, Specificity::new(0, 0, 0))
}

fn consume<'a, T>(this: &mut &'a [T]) -> Option<&'a T> {
    let last = this.last()?;

    if let Some(parts) = this.get(..this.len() - 1) {
        *this = parts;
    }

    Some(last)
}

/// Returns true when the given node matches the part(s)
fn match_selector_parts<C: HasDocument>(doc: &C::Document, node_id: NodeId, mut parts: &[CssSelectorPart]) -> bool {
    let mut next_current_node = doc.node_by_id(node_id);
    if next_current_node.is_none() {
        return false;
    }

    while let Some(part) = consume(&mut parts) {
        let Some(current_node) = next_current_node else {
            return false;
        };

        if current_node.is_root() {
            return false;
        }

        if !match_selector_part::<C>(part, current_node, doc, &mut next_current_node, &mut parts) {
            return false;
        }

        // We have matched this part, so we move up the chain
        // let binding = document.get();
        // next_current_node = binding.parent_node(current_node);
    }

    // All parts of the selector have matched
    true
}

fn match_selector_part<'a, C: HasDocument>(
    part: &CssSelectorPart,
    current_node: &C::Node,
    doc: &'a C::Document,
    next_node: &mut Option<&'a C::Node>,
    parts: &mut &[CssSelectorPart],
) -> bool {
    match part {
        CssSelectorPart::Universal => {
            // '*' always matches any selector
            true
        }
        CssSelectorPart::Type(name) => {
            if !current_node.is_element_node() {
                return false;
            }
            *name == current_node.get_element_data().unwrap().name()
        }
        CssSelectorPart::Class(name) => {
            if !current_node.is_element_node() {
                return false;
            }
            current_node.get_element_data().unwrap().classlist().contains(name)
        }
        CssSelectorPart::Id(name) => {
            if !current_node.is_element_node() {
                return false;
            }
            current_node
                .get_element_data()
                .unwrap()
                .attributes()
                .get("id")
                .unwrap_or(&String::new())
                == name
        }
        CssSelectorPart::Attribute(attr) => {
            let wanted_attr_name = &attr.name;

            if current_node.get_element_data().is_none() {
                return false;
            }

            let element_data = current_node.get_element_data().unwrap();
            if !element_data.attributes().contains_key(wanted_attr_name) {
                return false;
            }

            let mut wanted_attr_value = &attr.value;
            let mut got_attr_value = element_data.attributes().get(wanted_attr_name).unwrap();

            let mut _wanted_buf = String::new(); //Two buffers, so we don't need to clone the value if we match case-sensitive
            let mut _got_buf = String::new();
            // If we need to match case-insensitive, just convert everything to lowercase for comparison
            if attr.case_insensitive {
                _wanted_buf = wanted_attr_name.cow_to_lowercase().to_string();
                _got_buf = got_attr_value.cow_to_lowercase().to_string();

                wanted_attr_value = &_wanted_buf;
                got_attr_value = &_got_buf;
            }

            match attr.matcher {
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
                    wanted_attr_value.split_whitespace().any(|s| s == got_attr_value)
                }
                MatcherType::DashMatch => {
                    // Exact value or value followed by a hyphen
                    got_attr_value == wanted_attr_value || got_attr_value.starts_with(&format!("{wanted_attr_value}-"))
                }
                MatcherType::PrefixMatch => {
                    // Starts with
                    got_attr_value.starts_with(wanted_attr_value)
                }
                MatcherType::SuffixMatch => {
                    // Ends with
                    got_attr_value.ends_with(wanted_attr_value)
                }
                MatcherType::SubstringMatch => {
                    // Contains
                    got_attr_value.contains(wanted_attr_value)
                }
            }
        }
        CssSelectorPart::PseudoClass(_name) => {
            // @Todo: implement pseudo classes
            false
        }
        CssSelectorPart::PseudoElement(_name) => {
            // @Todo: implement pseudo elements
            false
        }
        CssSelectorPart::Combinator(combinator) => {
            // We don't have the descendant combinator (space), as this is the default behaviour
            match combinator {
                Combinator::Descendant => {
                    // let handle_clone = handle.clone();

                    let Some(mut parent_id) = current_node.parent_id() else {
                        return false;
                    };

                    let last = consume(parts);
                    let Some(last) = last else {
                        return false;
                    };

                    loop {
                        let Some(parent) = doc.node_by_id(parent_id) else {
                            return false;
                        };

                        *next_node = Some(parent);

                        if match_selector_part::<C>(last, parent, doc, next_node, parts) {
                            return true;
                        }

                        let Some(p) = parent.parent_id() else {
                            return false;
                        };

                        parent_id = p;
                    }
                }
                Combinator::Child => {
                    // Child combinator. Only matches the direct child
                    let Some(parent) = current_node.parent_id() else {
                        return false;
                    };

                    let last = consume(parts);

                    let Some(last) = last else {
                        return false;
                    };

                    let Some(parent) = doc.node_by_id(parent) else {
                        return false;
                    };

                    *next_node = Some(parent);

                    match_selector_part::<C>(last, parent, doc, next_node, parts)
                }
                Combinator::NextSibling => {
                    let parent_node = doc.node_by_id(current_node.parent_id().unwrap());
                    let Some(children) = parent_node.map(gosub_interface::node::Node::children) else {
                        return false;
                    };

                    let Some(my_index) = children
                        .iter()
                        .find_position(|c| **c == current_node.id())
                        .map(|(i, _)| i)
                    else {
                        return false;
                    };

                    if my_index == 0 {
                        return false;
                    }

                    let Some(prev_id) = children.get(my_index - 1).copied() else {
                        return false;
                    };

                    let Some(last) = consume(parts) else {
                        return false;
                    };

                    let Some(prev) = doc.node_by_id(prev_id) else {
                        return false;
                    };

                    *next_node = Some(prev);

                    match_selector_part::<C>(last, prev, doc, next_node, parts)
                }
                Combinator::SubsequentSibling => {
                    let parent_node = doc.node_by_id(current_node.parent_id().unwrap());
                    let Some(children) = parent_node.map(gosub_interface::node::Node::children) else {
                        return false;
                    };

                    let Some(last) = consume(parts) else {
                        return false;
                    };

                    for child in children {
                        if *child == current_node.id() {
                            break;
                        }

                        let Some(child) = doc.node_by_id(*child) else {
                            continue;
                        };

                        if match_selector_part::<C>(last, child, doc, next_node, parts) {
                            return true;
                        }
                    }

                    false
                }
                Combinator::Namespace => {
                    let namespace = consume(parts);

                    let Some(namespace) = namespace else {
                        return false;
                    };

                    if *namespace == CssSelectorPart::Universal {
                        return true;
                    }

                    let CssSelectorPart::Type(namespace) = namespace else {
                        return false;
                    };

                    current_node.get_element_data().unwrap().is_namespace(namespace)
                }
                Combinator::Column => {
                    //TODO

                    false
                }
            }
        }
    }
}

/// A declarationProperty defines a single value for a property (color: red;). It consists of the value,
/// origin, importance, location and specificity of the declaration.
#[derive(Debug, Clone)]
pub struct DeclarationProperty {
    /// The actual value of the property (@todo: should this be a vec? or do we need to (re-)implement `CssValue::List`?)
    pub value: CssValue,
    /// Origin of the declaration (user stylesheet, author stylesheet etc.)
    pub origin: CssOrigin,
    /// Whether the declaration is !important
    pub important: bool,
    // @TODO: location should be a Location
    /// The location of the declaration in the stylesheet (name.css:123) or empty
    pub location: String,
    /// The specificity of the selector that declared this property
    pub specificity: Specificity,
}

impl DeclarationProperty {
    /// Priority of the declaration based on the origin and importance as defined in <https://developer.mozilla.org/en-US/docs/Web/CSS/Cascade>
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
        self.priority()
            .cmp(&other.priority())
            .then_with(|| self.specificity.cmp(&other.specificity))
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
    pub inherited: CssValue,
}

impl CssProperty {
    #[must_use]
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
            inherited: CssValue::None,
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
        self.declared.iter().max().map(|v| v.value.clone())
    }

    fn find_specified_value(&self) -> CssValue {
        self.cascaded.as_ref().unwrap_or(&self.inherited).clone()
    }

    fn find_computed_value(&self) -> CssValue {
        if self.specified != CssValue::None {
            return self.specified.clone();
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

    // /// Returns true if the given property is a shorthand property (ie: border, margin etc.)
    #[must_use]
    pub fn is_shorthand(&self) -> bool {
        let defs = get_css_definitions();
        match defs.find_property(&self.name) {
            Some(def) => def.expanded_properties().len() > 1,
            None => false,
        }
    }

    /// Returns the list of properties from a shorthand property, or just the property itself if it isn't a shorthand property.
    #[must_use]
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
            .map(super::property_definitions::PropertyDefinition::initial_value)
    }
}

impl From<CssValue> for CssProperty {
    fn from(value: CssValue) -> Self {
        let mut this = Self::new("unknown");

        this.declared = vec![DeclarationProperty {
            location: String::new(),
            important: false,
            value,
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
        }];

        this.calculate_value();

        this
    }
}

impl From<CssValue> for DeclarationProperty {
    fn from(value: CssValue) -> Self {
        Self {
            location: String::new(),
            important: false,
            value,
            origin: CssOrigin::Author,
            specificity: Specificity::new(0, 0, 0),
        }
    }
}

impl Display for CssProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.actual, f)
    }
}

impl css3::CssProperty<Css3System> for CssProperty {
    fn compute_value(&mut self) {
        self.compute_value();
    }
    fn unit_to_px(&self) -> f32 {
        self.actual.unit_to_px()
    }

    fn as_string(&self) -> Option<&str> {
        if let CssValue::String(str) = &self.actual {
            Some(str)
        } else {
            None
        }
    }

    fn as_percentage(&self) -> Option<f32> {
        if let CssValue::Percentage(percent) = &self.actual {
            Some(*percent)
        } else {
            None
        }
    }

    fn as_unit(&self) -> Option<(f32, &str)> {
        if let CssValue::Unit(value, unit) = &self.actual {
            Some((*value, unit))
        } else {
            None
        }
    }

    fn as_color(&self) -> Option<(f32, f32, f32, f32)> {
        if let CssValue::Color(color) = &self.actual {
            Some((color.r, color.g, color.b, color.a))
        } else {
            None
        }
    }

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)> {
        self.actual.to_color().map(|color| (color.r, color.g, color.b, color.a))
    }

    fn as_number(&self) -> Option<f32> {
        if let CssValue::Number(num) = &self.actual {
            Some(*num)
        } else {
            None
        }
    }

    fn as_list(&self) -> Option<&[CssValue]> {
        if let CssValue::List(list) = &self.actual {
            Some(list)
        } else {
            None
        }
    }

    fn as_function(&self) -> Option<(&str, &[CssValue])> {
        if let CssValue::Function(name, args) = &self.actual {
            Some((name.as_str(), args))
        } else {
            None
        }
    }

    fn is_none(&self) -> bool {
        matches!(self.actual, CssValue::None)
    }
}

/// Map of all declared values for a single node. Note that these are only the defined properties, not
/// the non-existing properties.
#[derive(Debug)]
pub struct CssProperties {
    pub properties: HashMap<String, CssProperty>,
    pub dirty: bool,
}

impl Default for CssProperties {
    fn default() -> Self {
        Self::new()
    }
}

impl CssProperties {
    #[must_use]
    pub fn new() -> Self {
        Self {
            properties: HashMap::new(),
            dirty: true,
        }
    }

    pub fn get(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.properties.get_mut(name)
    }
}

impl CssPropertyMap<Css3System> for CssProperties {
    fn insert_inherited(&mut self, name: &str, value: CssProperty) {
        self.properties.entry(name.to_string()).or_insert(value);
    }

    fn insert(&mut self, name: &str, value: CssProperty) {
        self.properties.insert(name.to_string(), value);
    }

    fn get(&self, name: &str) -> Option<&CssProperty> {
        self.properties.get(name)
    }

    fn get_mut(&mut self, name: &str) -> Option<&mut CssProperty> {
        self.properties.get_mut(name)
    }

    fn make_dirty(&mut self) {
        self.dirty = true;
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &CssProperty)> + '_ {
        self.properties.iter().map(|(k, v)| (k.as_str(), v))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut CssProperty)> + '_ {
        self.properties.iter_mut().map(|(k, v)| (k.as_str(), v))
    }

    fn make_clean(&mut self) {
        self.dirty = false;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use crate::colors::RgbColor;
    use crate::system::prop_is_inherit;

    use super::*;

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
            location: String::new(),
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
        assert!(!prop_is_inherit(&prop.name));
    }

    #[test]
    fn color_prop_test() {
        let mut prop = CssProperty::new("color");

        prop.declared.push(DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        });

        assert_eq!(prop.compute_value(), &CssValue::String("red".into()));
        assert!(!prop.is_shorthand());
        assert_eq!(prop.name, "color");
        assert_eq!(prop.get_initial_value(), Some(&CssValue::None).cloned());
        assert!(prop_is_inherit(&prop.name));
    }

    #[test]
    fn compare_declared() {
        let a = DeclarationProperty {
            value: CssValue::String("red".into()),
            origin: CssOrigin::Author,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        };
        let b = DeclarationProperty {
            value: CssValue::String("blue".into()),
            origin: CssOrigin::UserAgent,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        };
        let c = DeclarationProperty {
            value: CssValue::String("green".into()),
            origin: CssOrigin::User,
            important: false,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        };
        let d = DeclarationProperty {
            value: CssValue::String("yellow".into()),
            origin: CssOrigin::Author,
            important: true,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        };
        let e = DeclarationProperty {
            value: CssValue::String("orange".into()),
            origin: CssOrigin::UserAgent,
            important: true,
            location: String::new(),
            specificity: Specificity::new(1, 0, 0),
        };
        let f = DeclarationProperty {
            value: CssValue::String("purple".into()),
            origin: CssOrigin::User,
            important: true,
            location: String::new(),
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
        assert!(!prop_is_inherit(&prop.name));

        let prop = CssProperty::new("color");
        assert!(prop_is_inherit(&prop.name));

        let prop = CssProperty::new("font");
        assert!(prop_is_inherit(&prop.name));

        let prop = CssProperty::new("border-top-color");
        assert!(!prop_is_inherit(&prop.name));
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
