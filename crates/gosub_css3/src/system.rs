use crate::functions::attr::resolve_attr;
use crate::functions::calc::resolve_calc;
use crate::functions::var::resolve_var;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::shorthands::FixList;
use crate::matcher::styling::{match_selector, CssProperties, CssProperty, DeclarationProperty};
use crate::stylesheet::{CssDeclaration, CssValue, Specificity};
use crate::{load_default_useragent_stylesheet, Css3};
use gosub_shared::document::DocumentHandle;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use gosub_shared::traits::css3::{CssOrigin, CssPropertyMap, CssSystem};
use gosub_shared::traits::document::Document;
use gosub_shared::traits::node::{ElementDataType, Node, TextDataType};
use gosub_shared::traits::render_tree::{RenderTree, RenderTreeNode};
use gosub_shared::traits::ParserConfig;
use log::warn;
use std::slice;

#[derive(Debug, Clone)]
pub struct Css3System;

impl CssSystem for Css3System {
    type Stylesheet = crate::stylesheet::CssStylesheet;

    type PropertyMap = CssProperties;

    type Property = CssProperty;

    fn parse_str(str: &str, config: ParserConfig, origin: CssOrigin, url: &str) -> CssResult<Self::Stylesheet> {
        Css3::parse_str(str, config, origin, url)
    }

    fn properties_from_node<D: Document<Self>>(
        node: &D::Node,
        sheets: &[Self::Stylesheet],
        handle: DocumentHandle<D, Self>,
        id: NodeId,
    ) -> Option<Self::PropertyMap> {
        let mut css_map_entry = CssProperties::new();

        if node_is_unrenderable::<D, Self>(node) {
            return None;
        }

        let definitions = get_css_definitions();

        let mut fix_list = FixList::new();

        for sheet in sheets {
            for rule in &sheet.rules {
                for selector in rule.selectors().iter() {
                    let (matched, specificity) = match_selector(DocumentHandle::clone(&handle), id, selector);

                    if !matched {
                        continue;
                    }

                    // Selector matched, so we add all declared values to the map
                    for declaration in rule.declarations().iter() {
                        // Step 1: find the property in our CSS definition list
                        let Some(definition) = definitions.find_property(&declaration.property) else {
                            // If not found, we skip this declaration
                            warn!("Definition is not found for property {:?}", declaration.property);
                            continue;
                        };

                        let value = resolve_functions(&declaration.value, node, handle.clone());

                        let match_value = if let CssValue::List(value) = &value {
                            &**value
                        } else {
                            slice::from_ref(&value)
                        };

                        // Check if the declaration matches the definition and return the "expanded" order
                        let res = definition.matches_and_shorthands(match_value, &mut fix_list);
                        if !res {
                            warn!("Declaration does not match definition: {:?}", declaration);
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

                        // create property for the given values
                        let property_name = declaration.property.clone();
                        let decl = CssDeclaration {
                            property: property_name.to_string(),
                            value,
                            important: declaration.important,
                        };

                        add_property_to_map(&mut css_map_entry, sheet, specificity.clone(), &decl);
                    }
                }
            }
        }

        fix_list.resolve_nested(definitions);

        fix_list.apply(&mut css_map_entry);

        Some(css_map_entry)
    }

    fn inheritance<T: RenderTree<Self>>(tree: &mut T) {
        Self::resolve_inheritance(tree, tree.root(), &Vec::new());
    }

    fn load_default_useragent_stylesheet() -> Self::Stylesheet {
        load_default_useragent_stylesheet()
    }
}

impl Css3System {
    fn resolve_inheritance<T: RenderTree<Self>>(
        tree: &mut T,
        node_id: T::NodeId,
        inherit_props: &Vec<(String, CssValue)>,
    ) {
        let Some(current_node) = tree.get_node_mut(node_id) else {
            return;
        };

        for prop in inherit_props {
            let mut p = CssProperty::new(prop.0.as_str());

            p.inherited = prop.1.clone();

            current_node.props_mut().insert_inherited(prop.0.as_str(), p);
        }

        let mut inherit_props = inherit_props.clone();

        'props: for (name, prop) in &mut current_node.props_mut().iter_mut() {
            prop.compute_value();

            let value = prop.actual.clone();

            if prop_is_inherit(name) {
                for (k, v) in &mut inherit_props {
                    if k == name {
                        *v = value;
                        continue 'props;
                    }
                }

                inherit_props.push((name.to_owned(), value));
            }
        }

        let Some(children) = tree.get_children(node_id) else {
            return;
        };

        for child in children {
            Self::resolve_inheritance(tree, child, &inherit_props);
        }
    }
}

pub fn prop_is_inherit(name: &str) -> bool {
    get_css_definitions()
        .find_property(name)
        .map(|def| def.inherited)
        .unwrap_or(false)
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

pub fn node_is_unrenderable<D: Document<C>, C: CssSystem>(node: &D::Node) -> bool {
    // There are more elements that are not renderable, but for now we only remove the most common ones

    const REMOVABLE_ELEMENTS: [&str; 6] = ["head", "script", "style", "svg", "noscript", "title"];

    if let Some(element_data) = node.get_element_data() {
        if REMOVABLE_ELEMENTS.contains(&element_data.name()) {
            return true;
        }
    }

    if let Some(text_data) = &node.get_text_data() {
        if text_data.value().chars().all(|c| c.is_whitespace()) {
            return true;
        }
    }

    false
}

pub fn resolve_functions<D: Document<C>, C: CssSystem>(
    value: &CssValue,
    node: &D::Node,
    handle: DocumentHandle<D, C>,
) -> CssValue {
    fn resolve<D: Document<C>, C: CssSystem>(val: &CssValue, node: &D::Node, doc: &D) -> CssValue {
        match val {
            CssValue::Function(func, values) => {
                let resolved = match func.as_str() {
                    "calc" => resolve_calc(values),
                    "attr" => resolve_attr(values, node),
                    "var" => resolve_var(values, doc, node),
                    _ => vec![val.clone()],
                };

                CssValue::List(resolved)
            }
            _ => val.clone(),
        }
    }

    let doc = handle.get();

    if let CssValue::List(list) = value {
        let resolved = list.iter().map(|val| resolve(val, node, &*doc)).collect();
        CssValue::List(resolved)
    } else {
        resolve(value, node, &*doc)
    }
}
