use crate::functions::attr::resolve_attr;
use crate::functions::var::resolve_var;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::shorthands::FixList;
use crate::matcher::styling::{match_selector, CssProperties, CssProperty, DeclarationProperty};
use crate::stylesheet::{CssDeclaration, CssValue, Specificity};
use crate::{load_default_useragent_stylesheet, Css3};
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssSystem};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use std::slice;

/// Strip a vendor prefix (-webkit-, -moz-, -ms-, -o-) from a CSS keyword, returning
/// the unprefixed form. E.g. "-webkit-match-parent" → "match-parent".
fn strip_vendor_prefix(s: &str) -> &str {
    for prefix in &["-webkit-", "-moz-", "-ms-", "-o-"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return rest;
        }
    }
    s
}

/// Recursively normalize vendor-prefixed string values to their standard form.
fn normalize_vendor_prefixes(value: CssValue) -> CssValue {
    match value {
        CssValue::String(s) => CssValue::String(strip_vendor_prefix(&s).to_string()),
        CssValue::List(values) => CssValue::List(values.into_iter().map(normalize_vendor_prefixes).collect()),
        other => other,
    }
}

#[derive(Debug, Clone)]
pub struct Css3System;

impl CssSystem for Css3System {
    type Stylesheet = crate::stylesheet::CssStylesheet;

    type PropertyMap = CssProperties;

    type Property = CssProperty;
    type Value = CssValue;

    fn parse_str(str: &str, config: ParserConfig, origin: CssOrigin, url: &str) -> CssResult<Self::Stylesheet> {
        Css3::parse_str(str, config, origin, url)
    }

    fn properties_from_node<C: HasDocument<CssSystem = Self>>(
        doc: &C::Document,
        id: NodeId,
        sheets: &[Self::Stylesheet],
    ) -> Option<Self::PropertyMap> {
        let mut css_map_entry = CssProperties::new();

        if node_is_unrenderable::<C>(doc, id) {
            return None;
        }

        let definitions = get_css_definitions();

        let mut fix_list = FixList::new();

        for sheet in sheets {
            for rule in &sheet.rules {
                for selector in rule.selectors() {
                    let (matched, specificity) = match_selector::<C>(doc, id, selector);

                    if !matched {
                        continue;
                    }

                    // Selector matched, so we add all declared values to the map
                    for declaration in rule.declarations() {
                        let value = resolve_functions::<C>(&declaration.value, doc, id);
                        // Normalize vendor-prefixed values (-webkit-X → X) so they match
                        // against the standard keyword definitions.
                        let value = normalize_vendor_prefixes(value);

                        // If the property has a definition, validate and expand shorthands.
                        // If not (e.g. margin-top, padding-bottom — longhand properties not yet
                        // in the definition list), insert the value directly without validation.
                        match definitions.find_property(&declaration.property) {
                            Some(definition) => {
                                let match_value = if let CssValue::List(value) = &value {
                                    &**value
                                } else {
                                    slice::from_ref(&value)
                                };

                                if !definition.matches_and_shorthands(match_value, &mut fix_list) {
                                    // Special-case: `background: <color>` — the full shorthand
                                    // syntax requires comma-separated layers and the simple
                                    // single-color form fails the complex syntax. Treat it as
                                    // `background-color: <color>` which the consumer expects.
                                    if declaration.property == "background" {
                                        if let CssValue::Color(_) = &value {
                                            add_property_to_map(
                                                &mut css_map_entry,
                                                sheet,
                                                specificity,
                                                &CssDeclaration {
                                                    property: "background-color".to_string(),
                                                    value,
                                                    important: declaration.important,
                                                },
                                            );
                                            continue;
                                        }
                                    }
                                    log::debug!("Declaration does not match definition: {declaration:?}");
                                    continue;
                                }

                                let value = if let CssValue::List(mut value) = value {
                                    if value.len() == 1 {
                                        value.pop().expect("unreachable")
                                    } else {
                                        CssValue::List(value)
                                    }
                                } else {
                                    value
                                };

                                add_property_to_map(
                                    &mut css_map_entry,
                                    sheet,
                                    specificity,
                                    &CssDeclaration {
                                        property: declaration.property.clone(),
                                        value,
                                        important: declaration.important,
                                    },
                                );
                            }
                            None => {
                                // No definition: pass the value through as-is so that properties
                                // like margin-top, padding-left, font-size etc. (which are valid
                                // CSS but happen not to have their own PropertyDefinition entry)
                                // still reach the style consumer.
                                let value = if let CssValue::List(mut v) = value {
                                    if v.len() == 1 {
                                        v.pop().expect("unreachable")
                                    } else {
                                        CssValue::List(v)
                                    }
                                } else {
                                    value
                                };
                                add_property_to_map(
                                    &mut css_map_entry,
                                    sheet,
                                    specificity,
                                    &CssDeclaration {
                                        property: declaration.property.clone(),
                                        value,
                                        important: declaration.important,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }

        fix_list.resolve_nested(definitions);

        fix_list.apply(&mut css_map_entry);

        Some(css_map_entry)
    }

    fn load_default_useragent_stylesheet() -> Self::Stylesheet {
        load_default_useragent_stylesheet()
    }
}

#[must_use]
pub fn prop_is_inherit(name: &str) -> bool {
    get_css_definitions()
        .find_property(name)
        .is_some_and(|def| def.inherited)
}

pub fn add_property_to_map(
    css_map_entry: &mut CssProperties,
    sheet: &crate::stylesheet::CssStylesheet,
    specificity: Specificity,
    declaration: &CssDeclaration,
) {
    let property_name = declaration.property.clone();
    // let entry = CssProperty::new(property_name.as_str());

    // If the property is a shorthand css property, we need fetch the individual properties
    // It's possible that need to recurse here as these individual properties can be shorthand as well
    // if entry.is_shorthand() {
    //     for property_name in entry.get_props_from_shorthand() {
    //         let decl = CssDeclaration {
    //             property: property_name.to_string(),
    //             value: declaration.value.clone(),
    //             important: declaration.important,
    //         };
    //
    //         add_property_to_map(css_map_entry, sheet, selector, &decl);
    //     }
    // }
    //
    let declaration = DeclarationProperty {
        // @todo: this seems wrong. We only get the first values from the declared values
        value: declaration.value.clone(),
        origin: sheet.origin,
        important: declaration.important,
        location: sheet.url.clone(),
        specificity,
    };

    if let std::collections::hash_map::Entry::Vacant(e) = css_map_entry.properties.entry(property_name.clone()) {
        // Generate new property in the css map
        let mut entry = CssProperty::new(property_name.as_str());
        entry.declared.push(declaration);
        e.insert(entry);
    } else {
        // Just add the declaration to the existing property
        let entry = css_map_entry.properties.get_mut(&property_name).unwrap();
        entry.declared.push(declaration);
    }
}

pub fn node_is_unrenderable<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    const REMOVABLE_ELEMENTS: [&str; 6] = ["head", "script", "style", "svg", "noscript", "title"];

    match doc.node_type(id) {
        NodeType::ElementNode => doc.tag_name(id).is_some_and(|name| REMOVABLE_ELEMENTS.contains(&name)),
        NodeType::TextNode => doc.text_value(id).is_some_and(|v| v.chars().all(char::is_whitespace)),
        _ => false,
    }
}

pub fn resolve_functions<C: HasDocument>(value: &CssValue, doc: &C::Document, id: NodeId) -> CssValue {
    fn resolve<C: HasDocument>(val: &CssValue, doc: &C::Document, id: NodeId) -> CssValue {
        match val {
            CssValue::Function(func, values) => {
                let resolved = match func.as_str() {
                    "attr" => resolve_attr::<C>(values, doc, id),
                    "var" => resolve_var::<C>(values, doc, id),
                    _ => vec![val.clone()],
                };

                CssValue::List(resolved)
            }
            _ => val.clone(),
        }
    }

    if let CssValue::List(list) = value {
        let resolved = list.iter().map(|val| resolve::<C>(val, doc, id)).collect();
        CssValue::List(resolved)
    } else {
        resolve::<C>(value, doc, id)
    }
}
