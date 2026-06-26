use crate::functions::attr::resolve_attr;
use crate::functions::math::resolve_math;
use crate::functions::var::resolve_var;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::shorthands::{FixList, FixListInfo};
use crate::matcher::styling::{match_selector, CssProperties, CssProperty, DeclarationProperty};
use crate::stylesheet::{CssDeclaration, CssStylesheet, CssValue, Specificity};
use crate::{load_default_useragent_stylesheet, Css3};
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssPropertyMap, CssSystem, HoverFingerprints};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use std::collections::HashMap;
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
        compute_properties::<C>(doc, id, sheets, None)
    }

    fn pseudo_properties_from_node<C: HasDocument<CssSystem = Self>>(
        doc: &C::Document,
        id: NodeId,
        sheets: &[Self::Stylesheet],
        pseudo: &str,
    ) -> Option<Self::PropertyMap> {
        // Only `::before` / `::after` generate boxes; ignore other pseudo-elements.
        if !matches!(pseudo, "before" | "after") {
            return None;
        }
        let map = compute_properties::<C>(doc, id, sheets, Some(pseudo))?;
        // A pseudo-element only generates a box when a matching rule sets `content`. With no
        // `content` declaration there is nothing to render, so report "no pseudo-element".
        if <CssProperties as CssPropertyMap<Css3System>>::get(&map, "content").is_none() {
            return None;
        }
        Some(map)
    }

    fn load_default_useragent_stylesheet() -> Self::Stylesheet {
        load_default_useragent_stylesheet()
    }

    fn hover_fingerprints(sheets: &[Self::Stylesheet]) -> HoverFingerprints {
        hover_fingerprints_impl(sheets)
    }
}

/// Shared style-collection core for both real elements (`pseudo == None`) and pseudo-elements
/// (`pseudo == Some("before"|"after")`). When matching a pseudo-element, selectors are matched
/// against the originating element `id` but only those carrying the matching `::pseudo` part apply.
fn compute_properties<C: HasDocument<CssSystem = Css3System>>(
    doc: &C::Document,
    id: NodeId,
    sheets: &[CssStylesheet],
    pseudo: Option<&str>,
) -> Option<CssProperties> {
    let mut css_map_entry = CssProperties::new();

    // The unrenderable check applies to real elements only; a pseudo-element is generated
    // content hanging off a (renderable) originating element.
    if pseudo.is_none() && node_is_unrenderable::<C>(doc, id) {
        return None;
    }

    let definitions = get_css_definitions();

    // Pass 1: collect all custom property values visible to this node (with inheritance).
    let custom_props = collect_custom_props::<C>(doc, id, sheets);

    let mut fix_list = FixList::new();

    for sheet in sheets {
        for rule in &sheet.rules {
            for selector in rule.selectors() {
                let (matched, specificity) = match_selector::<C>(doc, id, selector, pseudo);

                if !matched {
                    continue;
                }

                // Selector matched, so we add all declared values to the map
                for declaration in rule.declarations() {
                    // Custom property declarations are consumed by collect_custom_props;
                    // skip them here so they don't clutter the regular property map.
                    if declaration.property.starts_with("--") {
                        continue;
                    }
                    let value = resolve_functions::<C>(&declaration.value, doc, id, &custom_props);
                    // Normalize vendor-prefixed values (-webkit-X → X) so they match
                    // against the standard keyword definitions.
                    let value = normalize_vendor_prefixes(value);

                    // `content` carries arbitrary tokens (strings, `attr()`, counters,
                    // quotes) that the property-syntax matcher cannot validate — notably the
                    // empty string `content: ""`. Pass it through verbatim; the render
                    // pipeline resolves it into generated text itself.
                    if declaration.property == "content" {
                        add_property_to_map(
                            &mut css_map_entry,
                            sheet,
                            specificity,
                            &CssDeclaration {
                                property: "content".to_string(),
                                value,
                                important: declaration.important,
                            },
                        );
                        continue;
                    }

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

                            // Tag the expanded longhands with this declaration's cascade origin
                            // and specificity, so e.g. an author `margin: 0` outranks the UA
                            // `body { margin: 8px }` instead of losing to it on processing order.
                            fix_list.set_info(FixListInfo::new(
                                sheet.origin,
                                declaration.important,
                                sheet.url.clone(),
                                specificity,
                            ));

                            // Each CSS declaration starts with a fresh TRBL multiplier
                            // counter for this shorthand name. Without this reset, a prior
                            // rule's `margin: 0` (count→1) would corrupt a later rule's
                            // `margin: 0 auto` expansion (starting at multi=1 instead of 0).
                            fix_list.reset_multiplier(&declaration.property);
                            if !definition.matches_and_shorthands(match_value, &mut fix_list) {
                                // Special-case: the full `background` shorthand grammar
                                // (comma-separated `<bg-layer>` lists) is stricter than the
                                // matcher supports, so common forms like
                                // `background: url(x) no-repeat` or `background: #fff` fail
                                // validation and would be dropped entirely. Recover the parts
                                // the consumer understands — `background-image` (a `url()`)
                                // and `background-color` (a color) — and emit them as the
                                // corresponding longhands. Position/repeat/size are still
                                // ignored.
                                if declaration.property == "background" {
                                    let mut recovered = false;
                                    // `url(...)` or a `*-gradient(...)` both become the
                                    // `background-image` longhand the consumer reads.
                                    if let Some(image_value) =
                                        find_background_url(&value).or_else(|| find_background_gradient(&value))
                                    {
                                        add_property_to_map(
                                            &mut css_map_entry,
                                            sheet,
                                            specificity,
                                            &CssDeclaration {
                                                property: "background-image".to_string(),
                                                value: image_value,
                                                important: declaration.important,
                                            },
                                        );
                                        recovered = true;
                                    }
                                    if let Some(color_value) = find_background_color(&value) {
                                        add_property_to_map(
                                            &mut css_map_entry,
                                            sheet,
                                            specificity,
                                            &CssDeclaration {
                                                property: "background-color".to_string(),
                                                value: color_value,
                                                important: declaration.important,
                                            },
                                        );
                                        recovered = true;
                                    }
                                    if recovered {
                                        continue;
                                    }
                                }
                                log::debug!("Declaration does not match definition: {declaration:?}");
                                continue;
                            }

                            let value = if let CssValue::List(mut values) = value {
                                match values.pop() {
                                    Some(single) if values.is_empty() => single,
                                    Some(last) => {
                                        values.push(last);
                                        CssValue::List(values)
                                    }
                                    None => CssValue::List(values),
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
                            let value = if let CssValue::List(mut values) = value {
                                match values.pop() {
                                    Some(single) if values.is_empty() => single,
                                    Some(last) => {
                                        values.push(last);
                                        CssValue::List(values)
                                    }
                                    None => CssValue::List(values),
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

fn hover_fingerprints_impl(sheets: &[CssStylesheet]) -> HoverFingerprints {
    use crate::stylesheet::CssSelectorPart;

    let mut fp = HoverFingerprints::default();

    for sheet in sheets {
        for rule in &sheet.rules {
            for selector in &rule.selectors {
                for part_list in &selector.parts {
                    // Split the part list into compounds (groups between Combinators).
                    // :hover belongs to the compound it appears in; that compound's
                    // Type/Class/Id parts are the hover-subject fingerprint.
                    let mut compound: Vec<&CssSelectorPart> = Vec::new();
                    for part in part_list {
                        if matches!(part, CssSelectorPart::Combinator(_)) {
                            compound.clear();
                            continue;
                        }
                        compound.push(part);
                        if !matches!(part, CssSelectorPart::PseudoClass(n) if n == "hover") {
                            continue;
                        }
                        // Found :hover — classify this compound.
                        let mut specific = false;
                        for p in &compound {
                            match p {
                                CssSelectorPart::Type(t) => {
                                    fp.types.insert(t.clone());
                                    specific = true;
                                }
                                CssSelectorPart::Class(c) => {
                                    fp.classes.insert(c.clone());
                                    specific = true;
                                }
                                CssSelectorPart::Id(id) => {
                                    fp.ids.insert(id.clone());
                                    specific = true;
                                }
                                _ => {}
                            }
                        }
                        if !specific {
                            // Bare :hover or *:hover — everything is sensitive.
                            fp.has_universal = true;
                            return fp;
                        }
                    }
                }
            }
        }
    }

    fp
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

    css_map_entry
        .properties
        .entry(property_name.clone())
        .or_insert_with(|| CssProperty::new(property_name.as_str()))
        .declared
        .push(declaration);
}

pub fn node_is_unrenderable<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    const REMOVABLE_ELEMENTS: [&str; 6] = ["head", "script", "style", "svg", "noscript", "title"];

    match doc.node_type(id) {
        NodeType::ElementNode => doc.tag_name(id).is_some_and(|name| REMOVABLE_ELEMENTS.contains(&name)),
        NodeType::TextNode => doc.text_value(id).is_some_and(|v| v.chars().all(char::is_whitespace)),
        _ => false,
    }
}

/// Collects all custom property (`--*`) values visible to `id`, walking ancestors
/// root-first so that each element's own declarations override inherited ones.
fn collect_custom_props<C: HasDocument<CssSystem = Css3System>>(
    doc: &C::Document,
    id: NodeId,
    sheets: &[CssStylesheet],
) -> HashMap<String, CssValue> {
    let mut chain = vec![id];
    let mut cur = id;
    while let Some(parent) = doc.parent(cur) {
        chain.push(parent);
        cur = parent;
    }
    chain.reverse(); // root first — descendants override ancestors

    let mut custom_props: HashMap<String, CssValue> = HashMap::new();
    for node_id in chain {
        for sheet in sheets {
            for rule in &sheet.rules {
                for selector in rule.selectors() {
                    let (matched, _) = match_selector::<C>(doc, node_id, selector, None);
                    if !matched {
                        continue;
                    }
                    for decl in rule.declarations() {
                        if decl.property.starts_with("--") {
                            custom_props.insert(decl.property.clone(), decl.value.clone());
                        }
                    }
                }
            }
        }
    }
    custom_props
}

/// Recursively find the first `url(...)` function inside a (possibly nested/list) CSS value.
/// Used to recover `background-image` from a `background` shorthand that fails strict matching.
fn find_background_url(value: &CssValue) -> Option<CssValue> {
    match value {
        CssValue::Function(name, _) if name.eq_ignore_ascii_case("url") => Some(value.clone()),
        CssValue::List(list) => list.iter().find_map(find_background_url),
        _ => None,
    }
}

/// Recursively find the first `*-gradient(...)` function inside a (possibly nested/list)
/// CSS value. Used to recover the image part of a `background` shorthand whose full
/// `<bg-layer>` grammar the value matcher does not yet support.
fn find_background_gradient(value: &CssValue) -> Option<CssValue> {
    match value {
        CssValue::Function(name, _) if name.to_ascii_lowercase().ends_with("gradient") => Some(value.clone()),
        CssValue::List(list) => list.iter().find_map(find_background_gradient),
        _ => None,
    }
}

/// Recursively find the first color inside a (possibly nested/list) CSS value.
/// Used to recover `background-color` from a `background` shorthand. The `currentColor`
/// keyword is a valid color too; it is preserved as a string and resolved to the element's
/// `color` later in the render bridge.
fn find_background_color(value: &CssValue) -> Option<CssValue> {
    match value {
        CssValue::Color(_) => Some(value.clone()),
        CssValue::String(s) if s.eq_ignore_ascii_case("currentcolor") => Some(value.clone()),
        CssValue::List(list) => list.iter().find_map(find_background_color),
        _ => None,
    }
}

pub fn resolve_functions<C: HasDocument>(
    value: &CssValue,
    doc: &C::Document,
    id: NodeId,
    custom_props: &HashMap<String, CssValue>,
) -> CssValue {
    fn resolve<C: HasDocument>(
        val: &CssValue,
        doc: &C::Document,
        id: NodeId,
        custom_props: &HashMap<String, CssValue>,
    ) -> CssValue {
        match val {
            CssValue::Function(func, values) => {
                let resolved = match func.as_str() {
                    "attr" => resolve_attr::<C>(values, doc, id),
                    "var" => resolve_var(values, custom_props),
                    "clamp" | "min" | "max" => {
                        resolve_math(func, values).map_or_else(|| vec![val.clone()], |v| vec![v])
                    }
                    _ => vec![val.clone()],
                };

                CssValue::List(resolved)
            }
            _ => val.clone(),
        }
    }

    if let CssValue::List(list) = value {
        let resolved = list
            .iter()
            .map(|val| resolve::<C>(val, doc, id, custom_props))
            .collect();
        CssValue::List(resolved)
    } else {
        resolve::<C>(value, doc, id, custom_props)
    }
}
