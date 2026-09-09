use crate::functions::attr::resolve_attr;
use crate::functions::math::resolve_math;
use crate::functions::var::resolve_var;
use crate::matcher::index::ElementKeys;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::shorthands::{FixList, FixListInfo};
use crate::matcher::styling::{
    cascade_rank, match_selector, CssProperties, CssProperty, DeclarationProperty, ScopeContext, ScopeMatch,
};
use crate::stylesheet::{CssDeclaration, CssStylesheet, CssValue, Specificity};
use crate::{load_default_useragent_stylesheet, Css3};
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssPropertyMap, CssSystem, HoverFingerprints};
use gosub_interface::document::Document;
use gosub_interface::node::NodeType;
use gosub_shared::config::ParserConfig;
use gosub_shared::errors::CssResult;
use gosub_shared::node::NodeId;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::slice;
use std::sync::Arc;

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

/// Specificity of the `style` attribute: above any selector.
const INLINE_SPECIFICITY: Specificity = Specificity::new(u32::MAX, 0, 0);

fn inline_parser_config() -> ParserConfig {
    ParserConfig {
        ignore_errors: true,
        ..Default::default()
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
        parent: Option<&Self::PropertyMap>,
    ) -> Option<Self::PropertyMap> {
        compute_properties::<C>(doc, id, sheets, None, parent)
    }

    fn pseudo_properties_from_node<C: HasDocument<CssSystem = Self>>(
        doc: &C::Document,
        id: NodeId,
        sheets: &[Self::Stylesheet],
        pseudo: &str,
        owner: Option<&Self::PropertyMap>,
    ) -> Option<Self::PropertyMap> {
        // Only `::before` / `::after` generate boxes; ignore other pseudo-elements.
        if !matches!(pseudo, "before" | "after") {
            return None;
        }
        let map = compute_properties::<C>(doc, id, sheets, Some(pseudo), owner)?;
        // A pseudo-element only generates a box when a matching rule sets `content`. With no
        // `content` declaration there is nothing to render, so report "no pseudo-element".
        <CssProperties as CssPropertyMap<Css3System>>::get(&map, "content")?;
        Some(map)
    }

    fn resolve_imports(sheet: &mut Self::Stylesheet, fetch: &mut gosub_interface::css3::ImportFetcher<'_>) {
        crate::imports::resolve_imports(sheet, fetch);
    }

    fn set_stylesheet_scope(sheet: &mut Self::Stylesheet, scope: Option<NodeId>) {
        sheet.scope = scope;
    }

    fn style_environment_fingerprint(sheets: &[Self::Stylesheet]) -> Option<u64> {
        Some(style_environment_fingerprint_impl(sheets))
    }

    fn load_default_useragent_stylesheet() -> Self::Stylesheet {
        load_default_useragent_stylesheet()
    }

    fn hover_fingerprints(sheets: &[Self::Stylesheet]) -> HoverFingerprints {
        hover_fingerprints_impl(sheets)
    }
}

/// Hash the parts of the environment the cascade reads, so a caller can tell whether a
/// viewport change actually invalidates computed styles.
///
/// Two inputs matter. Media conditions: a resize only restyles if some `@media` condition
/// flipped. Viewport units: `vw`/`vh` resolve when a declaration is computed, so a sheet
/// using them is stale after any resize at all.
///
/// Distinct `MediaQueryList`s are shared by every rule in their block, so they are evaluated
/// once each by address rather than once per rule - on a real-world sheet that is ~590
/// evaluations instead of ~7500.
fn style_environment_fingerprint_impl(sheets: &[CssStylesheet]) -> u64 {
    use std::collections::HashSet;
    use std::hash::{Hash, Hasher};

    let env = crate::media_query::media_environment();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut seen: HashSet<usize> = HashSet::new();
    let mut uses_viewport_units = false;

    for sheet in sheets {
        uses_viewport_units |= sheet.uses_viewport_units;
        for rule in &sheet.rules {
            let Some(conditions) = &rule.media else {
                continue;
            };
            for list in conditions {
                // Hash each distinct condition once, in first-seen (rule) order, so the
                // result is deterministic across runs.
                if seen.insert(Arc::as_ptr(list) as usize) {
                    list.matches(&env).hash(&mut hasher);
                }
            }
        }
    }

    if uses_viewport_units {
        env.width.to_bits().hash(&mut hasher);
        env.height.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// Shared style-collection core for both real elements (`pseudo == None`) and pseudo-elements
/// (`pseudo == Some("before"|"after")`). When matching a pseudo-element, selectors are matched
/// against the originating element `id` but only those carrying the matching `::pseudo` part apply.
///
/// `inherited` is the parent's (or, for a pseudo-element, the originating element's) computed
/// map; custom properties are read from it rather than by re-matching every ancestor.
fn compute_properties<C: HasDocument<CssSystem = Css3System>>(
    doc: &C::Document,
    id: NodeId,
    sheets: &[CssStylesheet],
    pseudo: Option<&str>,
    inherited: Option<&CssProperties>,
) -> Option<CssProperties> {
    let mut css_map_entry = CssProperties::new();

    // The unrenderable check applies to real elements only; a pseudo-element is generated
    // content hanging off a (renderable) originating element.
    if pseudo.is_none() && node_is_unrenderable::<C>(doc, id) {
        return None;
    }

    let definitions = get_css_definitions();

    // Selector matching is the expensive part: consult only the rules the index says can
    // match this element, match those once, and keep the hits.
    let keys = ElementKeys {
        id: doc.attribute(id, "id"),
        classes: doc.attribute(id, "class").unwrap_or(""),
        tag: doc.tag_name(id),
    };
    let mut matched: Vec<(&CssStylesheet, &crate::stylesheet::CssRule, Specificity, u16)> = Vec::new();
    // Media conditions hold for the whole pass, so read the environment once rather than per
    // rule. Unconditional rules never look at it.
    let media_env = crate::media_query::media_environment();
    // Which tree this element lives in decides which sheets may reach it at all.
    let element_scope = tree_scope::<C>(doc, id);
    for sheet in sheets {
        // A sheet from another tree contributes nothing, except through the two selectors
        // that are defined to reach across (`:host`, `::slotted()`).
        let Some(scope) = sheet_scope_for::<C>(doc, id, element_scope, sheet) else {
            continue;
        };
        let depth = shadow_depth::<C>(doc, sheet.scope);
        for rule_idx in sheet.candidate_rules(&keys) {
            let rule = &sheet.rules[rule_idx];
            // Cheaper than selector matching, so it goes first: a rule inside a `@media` block
            // that does not apply to this device contributes nothing to the cascade.
            if !rule.media_matches(&media_env) {
                continue;
            }
            // A rule applies with the highest specificity among its matching selectors.
            let best = rule
                .selectors()
                .iter()
                .filter_map(|selector| match match_selector::<C>(doc, id, selector, pseudo, scope) {
                    (true, specificity) => Some(specificity),
                    (false, _) => None,
                })
                .max();
            if let Some(specificity) = best {
                matched.push((sheet, rule, specificity, depth));
            }
        }
    }

    // Custom properties: the parent's scope with this node's own declarations cascaded on
    // top (origin/importance rank, then specificity, later wins ties), resolved before any
    // `var()` is read. The map is only copied when the node actually changes something;
    // re-declaring the inherited value (the `* { --x: 0 }` reset pattern) shares the parent's.
    let inherited_custom = inherited.map(|map| Arc::clone(&map.custom)).unwrap_or_default();
    let mut own_custom: HashMap<&str, ((u8, u16, Specificity), &CssValue)> = HashMap::new();
    for (sheet, rule, specificity, depth) in &matched {
        for decl in rule.declarations() {
            if !decl.property.starts_with("--") {
                continue;
            }
            // Same ordering as the regular cascade: origin/importance, then the cross-tree
            // tiebreak, then specificity.
            let rank = (
                cascade_rank(sheet.origin, decl.important),
                tree_rank(*depth, decl.important),
                *specificity,
            );
            match own_custom.entry(decl.property.as_str()) {
                Entry::Occupied(mut slot) if slot.get().0 <= rank => {
                    slot.insert((rank, &decl.value));
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert((rank, &decl.value));
                }
            }
        }
    }
    // The `style` attribute cascades above every stylesheet rule; it is parsed here only
    // when it can carry a custom property (it usually cannot), through the real parser.
    let inline_sheet = pseudo
        .is_none()
        .then(|| doc.attribute(id, "style"))
        .flatten()
        .filter(|style| style.contains("--"))
        .and_then(|style| {
            Css3::parse_str(&format!("*{{{style}}}"), inline_parser_config(), CssOrigin::Author, "").ok()
        });
    if let Some(rule) = inline_sheet.as_ref().and_then(|sheet| sheet.rules.first()) {
        for decl in rule.declarations() {
            if !decl.property.starts_with("--") {
                continue;
            }
            // The `style` attribute belongs to the element's own tree, so depth 0 applies.
            let rank = (
                cascade_rank(CssOrigin::Author, decl.important),
                tree_rank(0, decl.important),
                INLINE_SPECIFICITY,
            );
            match own_custom.entry(decl.property.as_str()) {
                Entry::Occupied(mut slot) if slot.get().0 <= rank => {
                    slot.insert((rank, &decl.value));
                }
                Entry::Occupied(_) => {}
                Entry::Vacant(slot) => {
                    slot.insert((rank, &decl.value));
                }
            }
        }
    }
    let changes_scope = own_custom
        .iter()
        .any(|(name, (_, value))| inherited_custom.get(*name) != Some(*value));
    let custom_props = if changes_scope {
        let mut merged = (*inherited_custom).clone();
        merged.extend(
            own_custom
                .into_iter()
                .map(|(name, (_, value))| (name.to_string(), value.clone())),
        );
        Arc::new(merged)
    } else {
        inherited_custom
    };
    css_map_entry.custom = Arc::clone(&custom_props);

    let mut fix_list = FixList::new();

    for (sheet, rule, specificity, depth) in matched {
        // Selector matched, so we add all declared values to the map
        for declaration in rule.declarations() {
            // Custom property declarations were consumed above; keep them out of
            // the regular cascade.
            if declaration.property.starts_with("--") {
                continue;
            }
            let value = resolve_functions::<C>(&declaration.value, doc, id, &custom_props);
            // Normalize vendor-prefixed values (-webkit-X → X) so they match
            // against the standard keyword definitions.
            let value = normalize_vendor_prefixes(value);

            // `content` carries arbitrary tokens (strings, `attr()`, counters,
            // quotes) that the property-syntax matcher cannot validate - notably the
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
                    depth,
                );
                continue;
            }

            // If the property has a definition, validate and expand shorthands.
            // If not (e.g. margin-top, padding-bottom - longhand properties not yet
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
                        depth,
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
                        // the consumer understands - `background-image` (a `url()`)
                        // and `background-color` (a color) - and emit them as the
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
                                    depth,
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
                                    depth,
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
                        depth,
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
                        depth,
                    );
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
                        // Found :hover - classify this compound.
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
                            // Bare :hover or *:hover - everything is sensitive.
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
    shadow_depth: u16,
) {
    let property_name = declaration.property.clone();

    let declaration = DeclarationProperty {
        // @todo: this seems wrong. We only get the first values from the declared values
        value: declaration.value.clone(),
        origin: sheet.origin,
        important: declaration.important,
        location: sheet.url.clone(),
        specificity,
        shadow_depth,
    };

    css_map_entry
        .properties
        .entry(property_name.clone())
        .or_insert_with(|| CssProperty::new(property_name.as_str()))
        .declared
        .push(declaration);
}

/// The tree scope `id` lives in: the shadow root at the top of its ancestor chain, or `None`
/// when that chain reaches the document.
///
/// A shadow root has no parent, so the walk stops there by itself - the same property that
/// keeps a descendant combinator from crossing the boundary.
pub fn tree_scope<C: HasDocument>(doc: &C::Document, id: NodeId) -> Option<NodeId> {
    let mut root = id;
    while let Some(parent) = doc.parent(root) {
        root = parent;
    }
    (doc.node_type(root) == NodeType::ShadowRootNode).then_some(root)
}

/// How many shadow boundaries lie between `scope` and the document.
fn shadow_depth<C: HasDocument>(doc: &C::Document, scope: Option<NodeId>) -> u16 {
    let mut depth = 0u16;
    let mut current = scope;
    while let Some(root) = current {
        depth = depth.saturating_add(1);
        let Some(host) = doc.shadow_host(root) else {
            break;
        };
        current = tree_scope::<C>(doc, host);
    }
    depth
}

/// The cross-tree cascade tiebreak for a declaration at `depth`; mirrors
/// `DeclarationProperty::tree_rank`, for the custom-property cascade which ranks by hand.
fn tree_rank(depth: u16, important: bool) -> u16 {
    if important {
        depth
    } else {
        u16::MAX - depth
    }
}

/// Whether `sheet` may style `id` at all, and if so how it reaches it.
///
/// A sheet applies inside its own tree scope. A shadow tree's sheet also reaches one step
/// outwards, but only through the two selectors defined for it: `:host` onto the host, and
/// `::slotted()` onto the light-DOM children projected into its slots. User-agent sheets are
/// not scoped - they describe the engine's defaults for every element in the document.
fn sheet_scope_for<C: HasDocument>(
    doc: &C::Document,
    id: NodeId,
    element_scope: Option<NodeId>,
    sheet: &CssStylesheet,
) -> Option<ScopeContext> {
    if sheet.origin == CssOrigin::UserAgent || sheet.scope == element_scope {
        return Some(ScopeContext {
            mode: ScopeMatch::Same,
            tree: sheet.scope,
        });
    }

    // Different trees. The only sheets that may still reach are a shadow tree's own, and only
    // onto its host or onto what is projected into it.
    let tree = sheet.scope?;
    let host = doc.shadow_host(tree)?;

    if id == host {
        return Some(ScopeContext {
            mode: ScopeMatch::Host,
            tree: Some(tree),
        });
    }
    // A slottable is a direct child of the host. Whether it was actually assigned is left to
    // `::slotted()` itself - an unassigned node renders nowhere, so styling it changes nothing.
    if doc.parent(id) == Some(host) {
        return Some(ScopeContext {
            mode: ScopeMatch::Slotted,
            tree: Some(tree),
        });
    }
    None
}

/// Elements whose styles are never worth computing because they never render.
///
/// Careful: an element listed here gets *no* computed style at all, so a `display: none` rule in
/// the user-agent stylesheet can never apply to it. Anything named here must therefore also be
/// pruned by the render tree's own list, or it will render after all - `noscript` used to be
/// listed here and nowhere else, which is exactly how its raw text ended up on the page.
pub fn node_is_unrenderable<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    const REMOVABLE_ELEMENTS: [&str; 5] = ["head", "script", "style", "svg", "title"];

    match doc.node_type(id) {
        NodeType::ElementNode => doc.tag_name(id).is_some_and(|name| REMOVABLE_ELEMENTS.contains(&name)),
        NodeType::TextNode => doc.text_value(id).is_some_and(|v| v.chars().all(char::is_whitespace)),
        _ => false,
    }
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
        CssValue::Function(name, _) if name.cow_to_ascii_lowercase().ends_with("gradient") => Some(value.clone()),
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
        // Flatten each element's resolution back into this list. `resolve` wraps a function's
        // result in a `CssValue::List`, so without this a `var()`/`attr()` used *inside* a
        // multi-token value (e.g. `border: 1px solid var(--rule)`) would nest as
        // `[1px, solid, [color]]`. The `<color>` component of the shorthand matcher only sees a
        // top-level `Color`, so the nested list is dropped and the border falls back to black.
        // Splicing the inner tokens in keeps `border-color` (and any other shorthand part that
        // comes from a variable) matchable.
        let mut resolved = Vec::with_capacity(list.len());
        for val in list {
            match resolve::<C>(val, doc, id, custom_props) {
                CssValue::List(inner) => resolved.extend(inner),
                other => resolved.push(other),
            }
        }
        CssValue::List(resolved)
    } else {
        resolve::<C>(value, doc, id, custom_props)
    }
}
