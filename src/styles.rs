use crate::css3::parser_config::ParserConfig;
use crate::css3::Css3;
use crate::html5::node::{Node, NodeId};
use crate::html5::parser::document::{Document, DocumentHandle};
use crate::styles::converter::{
    convert_css_ast_to_rules, CssOrigin, CssSelector, CssSelectorPart, CssSelectorType,
    CssStylesheet, MatcherType, Specificity,
};
use core::fmt::Debug;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;

pub mod converter;
pub mod css_colors;
mod property_list;
mod shorthands;

/// A declarationProperty defines a single value for a property (color: red;). It consists of the value, origin, importance, location and
/// specificity of the declaration.
#[derive(Debug)]
pub struct DeclarationProperty {
    /// The actual value of the property
    pub value: String,
    /// Origin of the declaration (user stylesheet, author stylesheet etc)
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

///
#[derive(Debug)]
pub struct ValueEntry {
    /// List of all declared values for this property
    pub declared: Vec<DeclarationProperty>,
    /// Cascaded value
    pub cascaded: Option<String>,
    pub specified: String,
    pub computed: String,
    pub used: String,
    pub actual: String,
}

impl Default for ValueEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl ValueEntry {
    pub fn new() -> Self {
        Self {
            declared: Vec::new(),
            cascaded: None,
            specified: "".into(),
            computed: "".into(),
            used: "".into(),
            actual: "".into(),
        }
    }
}

/// Map of all declared values for a single node
pub type CssMapEntry = HashMap<String, ValueEntry>;

/// Map of all declared values for all nodes in the document
pub type CssMap = HashMap<NodeId, CssMapEntry>;

/// Style calculator will generate a declared values map for all nodes in the document based on the stylesheets given
pub struct StyleCalculator {
    stylesheets: Vec<CssStylesheet>,
    document: DocumentHandle,
    css_map: CssMap,
}

impl StyleCalculator {
    /// Creates a new style calculator for the given document
    pub fn new(document: DocumentHandle) -> Self {
        let mut sheets = vec![];
        for css in document.get().stylesheets.iter() {
            sheets.push(css.clone());
        }

        let mut this = Self {
            stylesheets: Vec::new(),
            document,
            css_map: CssMap::new(),
        };
        for sheet in sheets {
            this.add_stylesheet(sheet);
        }

        this
    }

    /// Adds another stylesheet to the calculator. Order is not important (@todo: is it?)
    pub fn add_stylesheet(&mut self, stylesheet: CssStylesheet) {
        self.stylesheets.push(stylesheet);
    }

    /// Extracts all declared values from the stylesheets and stores them in the calculator
    pub fn find_declared_values(&mut self) {
        println!("finding declared values");

        // Restart css map
        self.css_map = CssMap::new();

        // Iterate the complete document tree
        let tree_iterator = crate::html5::parser::document::TreeIterator::new(&self.document);
        for current_node_id in tree_iterator {
            let mut css_map_entry = CssMapEntry::new();

            let doc = self.document.get();
            let node = doc.get_node_by_id(current_node_id).expect("node not found");
            if !node.is_element() {
                continue;
            }

            // Iterate all stylesheets we have
            for sheet in self.stylesheets.iter() {
                // println!("Processing sheet: {:?} {}", sheet.origin, sheet.location);

                // We iterate over all rules in the stylesheet
                for rule in sheet.rules.iter() {
                    for selector in rule.selectors().iter() {
                        // println!("Checking rule selector: {:?}", selector);
                        if !self.match_selector(current_node_id, selector) {
                            continue;
                        }

                        // println!("+++ Matched rule selector: {:?}", selector);

                        for declaration in rule.declarations().iter() {
                            let property = declaration.property.clone();

                            let declaration = DeclarationProperty {
                                value: declaration.value.clone(),
                                origin: sheet.origin.clone(),
                                important: declaration.important,
                                location: "".into(),
                                specificity: selector.specificity(),
                            };

                            if let std::collections::hash_map::Entry::Vacant(e) =
                                css_map_entry.entry(property.clone())
                            {
                                let mut entry = ValueEntry::new();
                                entry.declared.push(declaration);
                                e.insert(entry);
                                // println!("+++ Created new entry");
                            } else {
                                let entry = css_map_entry.get_mut(&property).unwrap();
                                entry.declared.push(declaration);
                                // println!("+++ Adding to existing entry");
                            }

                            // if css_map_entry.contains_key(&property) {
                            //     let entry = css_map_entry.get_mut(&property).unwrap();
                            //     entry.declared.push(declaration);
                            //     // println!("+++ Adding to existing entry");
                            // } else {
                            //     let mut entry = ValueEntry::new();
                            //     entry.declared.push(declaration);
                            //     css_map_entry.insert(property, entry);
                            //     // println!("+++ Created new entry");
                            // }
                        }
                    }
                }
                // css_node_map.insert(current_node_id, declaration_map);
            }

            self.css_map.insert(current_node_id, css_map_entry);
        }
    }

    /// Orders all declared values and finds the cascaded values
    pub fn find_cascaded_values(&mut self) {
        // println!("finding cascaded values");

        for (_, css_map_entry) in self.css_map.iter_mut() {
            // println!("Node: {:?}", node_id);
            for (_, entry) in css_map_entry.iter_mut() {
                // println!("  Property: {:?}:   Declared {}", property, entry.declared.len());

                // Sort on origin and importance
                entry.declared.sort();

                // sort on specificity
                entry.declared.sort_by(|a, b| {
                    if a.priority() != b.priority() {
                        return Ordering::Equal;
                    }
                    a.specificity.cmp(&b.specificity)
                });

                // @todo: sort on scoping proximity

                // order of appearance in the stylesheet. We use the last entry as the cascaded value
                entry.cascaded = entry.declared.last().map(|d| d.value.clone());

                // for declaration in entry.declared.iter() {
                //     println!("    - ({}) {:?}", declaration.priority(), declaration);
                // }
                // println!("    - Cascaded {:?}", entry.cascaded);
            }
        }
    }

    /// Returns the list of all properties (cascaded values for now) for the given node
    pub fn get_properties(&self, node_id: NodeId) -> HashMap<String, String> {
        let mut props = HashMap::new();

        if let Some(entry) = self.css_map.get(&node_id) {
            for (k, v) in entry {
                props.insert(k.clone(), v.cascaded.clone().unwrap_or("".into()));
            }
        }

        props
    }

    // Matches a complete selector (all parts) against the given node(id)
    fn match_selector(&self, node_id: NodeId, selector: &CssSelector) -> bool {
        let mut parts = selector.parts.clone();
        parts.reverse();
        self.match_selector_part(node_id, &mut parts)
    }

    /// Returns true when the given node matches the part(s)
    fn match_selector_part(
        &self,
        node_id: NodeId,
        selector_parts: &mut Vec<CssSelectorPart>,
    ) -> bool {
        let binding = self.document.get();
        let mut next_current_node = Some(binding.get_node_by_id(node_id).expect("node not found"));

        while !selector_parts.is_empty() {
            if next_current_node.is_none() {
                return false;
            }
            let current_node = next_current_node.expect("current_node not found");

            let part = selector_parts.remove(0);

            // println!("Checking node {:?} against part: {:?}", current_node.as_element().name, part);

            match part.type_ {
                CssSelectorType::Universal => {
                    // '*' always matches any selector
                }
                CssSelectorType::Type => {
                    if part.value != current_node.as_element().name {
                        return false;
                    }
                }
                CssSelectorType::Class => {
                    if !current_node.as_element().classes.contains(&part.value) {
                        return false;
                    }
                }
                CssSelectorType::Id => {
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
                    let mut wanted_attr_value = part.value.clone();
                    let mut got_attr_value = current_node
                        .get_attribute(&wanted_attr_name)
                        .unwrap_or(&"".to_string())
                        .to_string();

                    if !current_node.has_attribute(&wanted_attr_name) {
                        return false;
                    }

                    // If we need to match case-insensitive, just convert everything to lowercase for comparison
                    if part.flags.eq_ignore_ascii_case("i") {
                        wanted_attr_value = wanted_attr_value.to_lowercase();
                        got_attr_value = got_attr_value.to_lowercase();
                    };

                    match part.matcher {
                        MatcherType::None => {
                            // Just the presence of the attribute is enough
                            return true;
                        }
                        MatcherType::Equals => {
                            // Exact match
                            return wanted_attr_value == got_attr_value;
                        }
                        MatcherType::Includes => {
                            // Contains word
                            return wanted_attr_value
                                .split_whitespace()
                                .any(|s| s == got_attr_value);
                        }
                        MatcherType::DashMatch => {
                            // Exact value or value followed by a hyphen
                            return got_attr_value == wanted_attr_value
                                || got_attr_value.starts_with(&format!("{}-", wanted_attr_value));
                        }
                        MatcherType::PrefixMatch => {
                            // Starts with
                            return got_attr_value.starts_with(&wanted_attr_value);
                        }
                        MatcherType::SuffixMatch => {
                            // Ends with
                            return got_attr_value.ends_with(&wanted_attr_value);
                        }
                        MatcherType::SubstringMatch => {
                            // Contains
                            return got_attr_value.contains(&wanted_attr_value);
                        }
                    }
                }
                CssSelectorType::PseudoClass => {
                    // panic!("Pseudo selectors not implemented")
                    if part.value == "link" {
                        return false;
                    }
                    return false;
                }
                CssSelectorType::PseudoElement => {
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
                            if !self.match_selector_part(current_node.id, selector_parts) {
                                // we insert the combinator back so we the next loop will match against the parent node
                                selector_parts.insert(0, part);
                            }
                        }
                        ">" => {
                            // Child combinator. Only matches the direct child
                            if !self.match_selector_part(current_node.id, selector_parts) {
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
            next_current_node = parent_node(&binding, current_node);
        }

        // All parts of the selector have matched
        true
    }
}

/// Returns the parent node of the given node, or None when no parent is found
fn parent_node<'b>(doc: &'b Document, node: &'b Node) -> Option<&'b Node> {
    // Find the next element node in the parent chain. Will return None if we hit the root of the chain
    let mut cur_node = node;

    loop {
        let node_id = cur_node.parent;
        node_id?;

        cur_node = doc
            .get_node_by_id(node_id.expect("node_id"))
            .expect("node not found");
        if cur_node.is_element() {
            return Some(cur_node);
        }
    }
}

/// Loads the default user agent stylesheet
pub fn load_default_useragent_stylesheet() -> anyhow::Result<CssStylesheet> {
    // @todo: we should be able to browse to gosub://useragent.css and see the actual useragent css file
    let location = "gosub://useragent.css";
    let config = ParserConfig {
        source: Some(String::from(location)),
        ignore_errors: true,
        ..Default::default()
    };

    let css =
        fs::read_to_string("resources/useragent.css").expect("Could not load useragent stylesheet");
    let css_ast = Css3::parse(css.as_str(), config).expect("Could not parse useragent stylesheet");

    convert_css_ast_to_rules(&css_ast, CssOrigin::UserAgent, location)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_declared() {
        let a = DeclarationProperty {
            value: "red".into(),
            origin: CssOrigin::Author,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let b = DeclarationProperty {
            value: "blue".into(),
            origin: CssOrigin::UserAgent,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let c = DeclarationProperty {
            value: "green".into(),
            origin: CssOrigin::User,
            important: false,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let d = DeclarationProperty {
            value: "yellow".into(),
            origin: CssOrigin::Author,
            important: true,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let e = DeclarationProperty {
            value: "orange".into(),
            origin: CssOrigin::UserAgent,
            important: true,
            location: "".into(),
            specificity: Specificity::new(1, 0, 0),
        };
        let f = DeclarationProperty {
            value: "purple".into(),
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
        assert!(c == c);
        assert!(d == d);
    }
}
