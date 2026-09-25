use crate::functions::attr::resolve_attr;
use crate::functions::var::{resolve_var, MAX_VAR_DEPTH};
use crate::matcher::bloom::{ancestor_filter, AncestorFilter};
use crate::matcher::expansion::{single_value, ExpandedDeclaration};
use crate::matcher::index::ElementKeys;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::property_ids::{LonghandId, PropertyId};
use crate::matcher::shorthands::{FixList, FixListInfo};
use crate::matcher::styling::{
    cascade_rank, match_selector, CssProperties, CssProperty, DeclarationProperty, ScopeContext, ScopeMatch,
    DEFAULT_FONT_SIZE_PX,
};
use crate::stylesheet::{reduce_function, CssDeclaration, CssStylesheet, CssValue, Specificity};
use crate::{load_default_useragent_stylesheet, load_quirks_useragent_stylesheet, Css3};
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

/// Where a custom property declaration sorts: the cascade's steps, in order, as one key.
type CustomRank = (u8, u16, bool, u32, Specificity);

/// A rule that matched the element, with everything the cascade needs to rank it.
struct MatchedRule<'a> {
    sheet: &'a CssStylesheet,
    rule: &'a crate::stylesheet::CssRule,
    /// The highest specificity among the rule's selectors that matched.
    specificity: Specificity,
    /// How many shadow boundaries deep the declaring sheet sits.
    depth: u16,
    /// The rule's cascade layer, as an index into its own sheet's list.
    layer: Option<u32>,
    /// Whether this is the element's `style` attribute rather than a stylesheet rule.
    attached: bool,
}

/// A layer rank as the cascade sorts it: unlayered is the top of the order for a normal
/// declaration and the bottom for an important one, and the layers run in opposite directions
/// for the two (css-cascade-5 §6.4.1).
fn layer_sort_key(layer: Option<u32>, important: bool) -> u32 {
    match (layer, important) {
        (None, false) => u32::MAX,
        (None, true) => 0,
        (Some(layer), false) => layer.saturating_add(1),
        (Some(layer), true) => u32::MAX.saturating_sub(layer).saturating_sub(1),
    }
}

thread_local! {
    /// The buffer [`compute_properties`] hands the selector index, kept between elements so
    /// that styling a page allocates it once rather than once per element and sheet. Taken
    /// out and put back rather than borrowed, so that a re-entrant call would merely get a
    /// buffer of its own instead of failing.
    static CANDIDATES: std::cell::Cell<Vec<usize>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Specificity of the `style` attribute: above any selector.
const INLINE_SPECIFICITY: Specificity = Specificity::new(u32::MAX, 0, 0);

fn inline_parser_config() -> ParserConfig {
    ParserConfig {
        ignore_errors: true,
        ..Default::default()
    }
}

/// How many parsed `style` attributes to keep per thread. A page that reaches this many
/// *distinct* attribute texts is one where the cache has stopped paying for itself, so the
/// table is emptied rather than grown or evicted piecemeal.
const INLINE_SHEET_CACHE_LIMIT: usize = 4096;

thread_local! {
    /// Parsed `style` attributes, by the attribute's text.
    ///
    /// The text is the whole input to the parse, so identical text gives an identical sheet:
    /// the cache is content-addressed, and script rewriting an attribute simply asks a
    /// different question. Nothing mutates a sheet once parsed, so one `Arc` serves every
    /// element that carries the same `style` - and, with it, one expansion of its declarations.
    ///
    /// Per thread, like the media environment: a style computation reads the thread's
    /// environment, so its results belong to that thread.
    static INLINE_SHEETS: std::cell::RefCell<HashMap<String, Arc<CssStylesheet>>> =
        std::cell::RefCell::new(HashMap::new());

    /// Parsed presentational hints, by the declaration text the document produced.
    ///
    /// A sibling of `INLINE_SHEETS` rather than the same table: the two are the same kind of
    /// thing - a declaration block parsed once per distinct text - but a page has a handful of
    /// distinct hint texts against thousands of `style` attributes, and sharing the table would
    /// let the attributes evict the hints.
    static HINT_SHEETS: std::cell::RefCell<HashMap<String, Arc<CssStylesheet>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Specificity of a presentational hint: zero, so any selector at all outranks it
/// (HTML §15.3.1).
const HINT_SPECIFICITY: Specificity = Specificity::new(0, 0, 0);

/// The `style` attribute as a one-rule stylesheet, so it can join the cascade like any other
/// rule. Parsed once per distinct attribute text.
fn inline_stylesheet(style: &str) -> Option<Arc<CssStylesheet>> {
    INLINE_SHEETS.with(|cache| {
        if let Some(sheet) = cache.borrow().get(style) {
            return Some(Arc::clone(sheet));
        }
        let sheet =
            Arc::new(Css3::parse_str(&format!("*{{{style}}}"), inline_parser_config(), CssOrigin::Author, "").ok()?);
        let mut cache = cache.borrow_mut();
        if cache.len() >= INLINE_SHEET_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(style.to_string(), Arc::clone(&sheet));
        Some(sheet)
    })
}

/// An element's presentational hints as a one-rule stylesheet, so they join the cascade like
/// any other rule.
///
/// The document writes them as CSS and the real parser reads them back, which is what keeps the
/// HTML mapping table out of here: this only has to rank what it is handed. Parsed once per
/// distinct text, of which a page has very few - every `<td>` in a table produces the same one.
fn hint_stylesheet(hints: &str) -> Option<Arc<CssStylesheet>> {
    HINT_SHEETS.with(|cache| {
        if let Some(sheet) = cache.borrow().get(hints) {
            return Some(Arc::clone(sheet));
        }
        let sheet =
            Arc::new(Css3::parse_str(&format!("*{{{hints}}}"), inline_parser_config(), CssOrigin::Author, "").ok()?);
        let mut cache = cache.borrow_mut();
        if cache.len() >= INLINE_SHEET_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(hints.to_string(), Arc::clone(&sheet));
        Some(sheet)
    })
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

    fn load_quirks_useragent_stylesheet() -> Option<Self::Stylesheet> {
        Some(load_quirks_useragent_stylesheet())
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

    // The element's presentational attributes, as CSS the document wrote. Declared before
    // `matched`, which borrows from it, and before the unrenderable check, which it survives.
    let hint_sheet = pseudo
        .is_none()
        .then(|| doc.presentational_hints(id))
        .flatten()
        .and_then(|hints| hint_stylesheet(&hints));

    // The unrenderable check applies to real elements only; a pseudo-element is generated
    // content hanging off a (renderable) originating element.
    //
    // A hint outlives it. What the check skips is *selector matching* - no rule is expected to
    // reach a `<head>` or a `<title>` - but an element's own attributes describe it whatever
    // the sheets say, and `<svg>` is on the list while still being laid out. That used to work
    // because the presentational attributes were applied to the computed style after the
    // cascade had run, so an element with no map at all still got them.
    let matches_rules = pseudo.is_some() || !node_is_unrenderable::<C>(doc, id);
    if !matches_rules && hint_sheet.is_none() {
        return None;
    }
    let sheets: &[CssStylesheet] = if matches_rules { sheets } else { &[] };

    let definitions = get_css_definitions();

    // Selector matching is the expensive part: consult only the rules the index says can
    // match this element, match those once, and keep the hits.
    let keys = ElementKeys {
        id: doc.attribute(id, "id"),
        classes: doc.attribute(id, "class").unwrap_or(""),
        tag: doc.tag_name(id),
        attributes: doc.attributes(id),
        pseudo,
    };
    // The `style` attribute, parsed as a one-rule stylesheet so it can join the cascade as a
    // rule like any other. Declared before `matched` so it outlives the borrows taken of it.
    //
    // It used to be parsed only when it contained a `--`, and only its custom properties were
    // read: the render pipeline layered the ordinary declarations on afterwards, outside the
    // cascade entirely. Anything else asking the cascade what an element computes to - which is
    // to say `getComputedStyle` - therefore could not see a single thing set through
    // `element.style`.
    let inline_sheet = matches_rules
        .then(|| doc.attribute(id, "style"))
        .flatten()
        .filter(|style| !style.trim().is_empty())
        .and_then(inline_stylesheet);

    let mut matched: Vec<MatchedRule<'_>> = Vec::new();
    // Media conditions hold for the whole pass, so read the environment once rather than per
    // rule. Unconditional rules never look at it.
    let media_env = crate::media_query::media_environment();
    // Which tree this element lives in decides which sheets may reach it at all.
    let element_scope = tree_scope::<C>(doc, id);
    // What the elements above this one carry, so that a selector requiring an ancestor class or
    // id that none of them has is dropped before the matcher walks the chain to find that out.
    //
    // Built at most once per element, and only when some candidate rule actually asks about an
    // ancestor: on a page whose sheets are all single-compound selectors the filter would answer
    // "maybe" whatever it held, and building it would be pure cost. `inherited` is what lets the
    // build be a copy and one node's keys rather than a walk of the whole chain.
    let known = inherited.and_then(|map| Some((map.node?, map.ancestors.as_ref()?)));
    let mut ancestors: Option<Arc<AncestorFilter>> = None;

    // Presentational hints go in first, and nothing else has been collected yet, so they take
    // the lowest document-order positions of the pass. That is where HTML §15.3.1 puts them:
    // author-origin declarations at specificity zero, ordered as if they stood at the start of
    // the first author sheet. Origin alone settles the user-agent sheet, which they beat, and
    // any author declaration for the same property ties on specificity at best and comes later
    // in the order, so it wins.
    if let Some((sheet, rule)) = hint_sheet.as_ref().and_then(|s| s.rules.first().map(|r| (s, r))) {
        matched.push(MatchedRule {
            sheet,
            rule,
            specificity: HINT_SPECIFICITY,
            depth: shadow_depth::<C>(doc, element_scope),
            layer: None,
            attached: false,
        });
    }

    // One buffer for the index lookups of every sheet, borrowed from the thread rather than
    // allocated: the list is read and forgotten inside the loop, so nothing but its capacity
    // needs to outlive an element.
    let mut candidates = CANDIDATES.take();
    for sheet in sheets {
        // A sheet from another tree contributes nothing, except through the two selectors
        // that are defined to reach across (`:host`, `::slotted()`).
        let Some(scope) = sheet_scope_for::<C>(doc, id, element_scope, sheet) else {
            continue;
        };
        let depth = shadow_depth::<C>(doc, sheet.scope);
        sheet.candidate_rules(&keys, &mut candidates);
        for &rule_idx in &candidates {
            let rule = &sheet.rules[rule_idx];
            // Cheaper than selector matching, so it goes first: a rule inside a `@media` block
            // that does not apply to this device contributes nothing to the cascade.
            if !rule.media_matches(&media_env) {
                continue;
            }
            // The filter is only worth having for a rule that asks about an ancestor, and the
            // first such rule is what builds it.
            let filter = rule
                .asks_about_ancestors()
                .then(|| &**ancestors.get_or_insert_with(|| ancestor_filter::<C>(doc, id, known)));
            // A rule applies with the highest specificity among its matching selectors.
            let best = rule
                .selectors()
                .iter()
                .filter_map(
                    |selector| match match_selector::<C>(doc, id, selector, pseudo, scope, filter) {
                        (true, specificity) => Some(specificity),
                        (false, _) => None,
                    },
                )
                .max();
            if let Some(specificity) = best {
                matched.push(MatchedRule {
                    sheet,
                    rule,
                    specificity,
                    depth,
                    layer: rule.layer,
                    attached: false,
                });
            }
        }
    }
    CANDIDATES.set(candidates);
    // Hand the filter on. The element below this one gets its own by copying this and adding
    // this element's keys, instead of walking and re-hashing the chain from the top.
    css_map_entry.node = Some(id);
    css_map_entry.ancestors = ancestors;

    // The `style` attribute outranks every selector, which `INLINE_SPECIFICITY` says. It belongs
    // to the element's own tree, so it ranks at that tree's depth rather than the document's.
    if let Some((sheet, rule)) = inline_sheet.as_ref().and_then(|s| s.rules.first().map(|r| (s, r))) {
        matched.push(MatchedRule {
            sheet,
            rule,
            specificity: INLINE_SPECIFICITY,
            depth: shadow_depth::<C>(doc, element_scope),
            layer: None,
            attached: true,
        });
    }

    // Where each cascade layer sorts, merged across the sheets of each origin. Built only when
    // some sheet actually declares one, which no page that does not use `@layer` ever does -
    // and the test for that comes first, so such a page does not even collect the sheets.
    let layer_order = sheets
        .iter()
        .any(|sheet| !sheet.layers.is_empty())
        .then(|| {
            let sheet_refs: Vec<&CssStylesheet> = sheets.iter().collect();
            crate::layers::LayerOrder::build(&sheet_refs)
        })
        .flatten();
    let layer_rank = |matched: &MatchedRule<'_>| -> Option<u32> {
        let order = layer_order.as_ref()?;
        let name = matched.sheet.layers.get(matched.layer? as usize)?;
        Some(order.rank(matched.sheet.origin, name))
    };

    // Custom properties: the parent's scope with this node's own declarations cascaded on
    // top (origin/importance rank, then specificity, later wins ties), resolved before any
    // `var()` is read. The map is only copied when the node actually changes something;
    // re-declaring the inherited value (the `* { --x: 0 }` reset pattern) shares the parent's.
    let inherited_custom = inherited.map(|map| Arc::clone(&map.custom)).unwrap_or_default();
    let mut own_custom: HashMap<&str, (CustomRank, &CssValue)> = HashMap::new();
    for matched_rule in &matched {
        let MatchedRule {
            sheet,
            rule,
            specificity,
            depth,
            ..
        } = matched_rule;
        for decl in rule.declarations() {
            if !decl.property.is_custom() {
                continue;
            }
            // Same ordering as the regular cascade: origin/importance, then the cross-tree
            // tiebreak, then element-attached, then layer, then specificity.
            let rank = (
                cascade_rank(sheet.origin, decl.important),
                tree_rank(*depth, decl.important),
                matched_rule.attached,
                layer_sort_key(layer_rank(matched_rule), decl.important),
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
    // The `style` attribute needs no pass of its own here: it is one of the rules in `matched`,
    // so the loop above already cascaded its custom properties at inline specificity.
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

    // Document-order position of each declaration, the cascade's last tiebreak. `matched` is in
    // stylesheet order and `declarations()` in source order, so a simple running counter is
    // exactly the order the author wrote.
    let mut order: u32 = 0;

    for matched_rule in matched {
        let MatchedRule {
            sheet,
            rule,
            specificity,
            depth,
            ..
        } = matched_rule;
        let layer = layer_rank(&matched_rule);
        let attached = matched_rule.attached;
        // Selector matched, so we add all declared values to the map
        for (declaration, expanded) in rule.declarations().iter().zip(rule.expanded()) {
            order += 1;
            match expanded {
                // Custom property declarations were consumed above; keep them out of the
                // regular cascade.
                ExpandedDeclaration::Custom => continue,
                // Unknown property, or a value its grammar rejects. Which of the two it was,
                // and why, was logged when the rule was expanded.
                ExpandedDeclaration::Invalid => continue,
                ExpandedDeclaration::Resolved { entries, important } => {
                    // The declaration and every longhand it expands to, already worked out.
                    // All that is left is the element's own cascade facts.
                    for (id, value) in entries {
                        push_declaration(
                            &mut css_map_entry,
                            *id,
                            value,
                            sheet,
                            *important,
                            specificity,
                            depth,
                            order,
                            layer,
                            attached,
                        );
                    }
                    continue;
                }
                // A substitution function reads the element or the environment, so what this
                // declaration says - and whether it says anything valid at all - is only known
                // here. It takes the per-element path below.
                ExpandedDeclaration::Pending => {}
            }
            let value = resolve_functions::<C>(&declaration.value, doc, id, &custom_props);

            // `content` used to be passed through here without validation, because its
            // grammar could not be matched against the tokens the parser produced - the empty
            // string of `::before { content: "" }` most of all. It can now, so it goes through
            // the same path as everything else and a `content: 10px` is dropped.
            match declaration
                .property
                .id()
                .and_then(|id| Some((id, definitions.definition(id)?)))
            {
                Some((id, definition)) => {
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
                        Arc::clone(&sheet.url),
                        specificity,
                        depth,
                        order,
                        layer,
                        attached,
                    ));

                    // Each CSS declaration starts with a fresh TRBL multiplier
                    // counter for this shorthand name. Without this reset, a prior
                    // rule's `margin: 0` (count→1) would corrupt a later rule's
                    // `margin: 0 auto` expansion (starting at multi=1 instead of 0).
                    fix_list.reset_multiplier(declaration.property.as_str());
                    if !definition.matches_and_shorthands(match_value, &mut fix_list) {
                        log::debug!("Declaration does not match definition: {declaration:?}");
                        continue;
                    }
                    // A shorthand sets every one of its longhands; the ones it left out are
                    // reset to their initial value.
                    fix_list.reset_unmentioned(definition, match_value, definitions);

                    push_declaration(
                        &mut css_map_entry,
                        id,
                        // This value was produced per element by substitution, so it is new
                        // here and gets its own allocation; it is then shared with the map.
                        &Arc::new(single_value(value)),
                        sheet,
                        declaration.important,
                        specificity,
                        depth,
                        order,
                        layer,
                        attached,
                    );
                }
                None => {
                    // A property this engine has no definition for is a property it does not
                    // support, and a declaration for one is invalid (css-syntax-3 §9). It is
                    // dropped rather than passed through: an unvalidated value reaching the
                    // style consumer is how `dsiplay: block` used to be recorded and answered
                    // by `getComputedStyle` as though it were a real declaration.
                    //
                    // The comment here used to say this path carried the common longhands,
                    // which have had their own definitions for a long time. What reaches it now
                    // is misspellings, properties from specs the definitions data does not
                    // cover, and the few `-internal-` names the user-agent sheet sets and
                    // nothing reads.
                    log::debug!("Unknown property, declaration dropped: {}", declaration.property);
                    continue;
                }
            }
        }
    }

    fix_list.resolve_nested(definitions);

    fix_list.apply(&mut css_map_entry);

    if let Some(parent) = inherited {
        css_map_entry.inherit_from(parent);
    }

    resolve_font_size_basis(&mut css_map_entry, inherited);

    Some(css_map_entry)
}

/// Work out what an `em` and a `rem` mean on this element, and tell every property.
///
/// `font-size` has to go first and is the only one measured against the *parent*: `font-size:
/// 2em` doubles what it inherits, not itself. Every other property then resolves against this
/// element's own size. The result is stored on the map so a child can read its parent's basis
/// without recomputing the parent's cascade.
fn resolve_font_size_basis(map: &mut CssProperties, inherited: Option<&CssProperties>) {
    let parent_px = inherited.map_or(DEFAULT_FONT_SIZE_PX, |parent| parent.font_size_px);
    // No parent map means no element above this one, so this is the root - and the root is what
    // a `rem` is measured against. Its own `font-size` is therefore the one declaration a `rem`
    // cannot refer to without circularity, so there it means the initial size.
    let parent_root_px = inherited.map_or(DEFAULT_FONT_SIZE_PX, |parent| parent.root_font_size_px);

    let own_px = match map.get_id_mut(FONT_SIZE) {
        Some(font_size) => {
            font_size.font_size_basis = parent_px;
            font_size.root_font_size_basis = parent_root_px;
            font_size.mark_dirty();
            match font_size.compute_value() {
                CssValue::Unit(px, unit) if unit.eq_ignore_ascii_case("px") => *px as f32,
                // A percentage font-size is a fraction of the *parent's* computed font-size
                // (css-fonts-4 §3.5), which is a basis this already has - unlike every other
                // percentage, which needs a containing block and so has to wait for layout.
                // `html { font-size: 62.5% }` is the idiom that makes 1rem equal 10px.
                #[expect(clippy::cast_possible_truncation, reason = "a font-size fits an f32")]
                CssValue::Percentage(pct) => parent_px * (*pct as f32) / 100.0,
                // A keyword (`larger`), or anything else this does not resolve: inheriting the
                // parent's size is closer than falling back to the initial one.
                _ => parent_px,
            }
        }
        // Undeclared, so inherited - which is what `font-size` does by default.
        None => parent_px,
    };
    map.font_size_px = own_px;

    let root_px = if inherited.is_some() { parent_root_px } else { own_px };
    map.root_font_size_px = root_px;

    for (id, property) in map.iter_ids_mut() {
        if id != FONT_SIZE {
            property.font_size_basis = own_px;
            property.root_font_size_basis = root_px;
            // The basis changed after the property was built, so any value computed before now
            // used the default and has to be recomputed.
            property.mark_dirty();
        }
    }
}

fn hover_fingerprints_impl(sheets: &[CssStylesheet]) -> HoverFingerprints {
    use crate::stylesheet::CssSelectorPart;

    let mut fp = HoverFingerprints::default();

    for sheet in sheets {
        for rule in &sheet.rules {
            for selector in &rule.selectors {
                for part_list in selector.complexes() {
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

/// `font-size` is the one property every other property's `em` resolves against.
const FONT_SIZE: PropertyId = PropertyId::Longhand(LonghandId::FontSize);

/// Whether the property `name` denotes inherits by default.
#[must_use]
pub fn prop_is_inherit(name: &str) -> bool {
    PropertyId::from_name(name).is_some_and(PropertyId::inherited)
}

#[allow(clippy::too_many_arguments, reason = "one cascade fact per argument")]
pub fn add_property_to_map(
    css_map_entry: &mut CssProperties,
    sheet: &crate::stylesheet::CssStylesheet,
    specificity: Specificity,
    declaration: &CssDeclaration,
    shadow_depth: u16,
    order: u32,
    layer: Option<u32>,
    attached: bool,
) {
    let Some(id) = declaration.property.id() else {
        return;
    };
    push_declaration(
        css_map_entry,
        id,
        &declaration.value,
        sheet,
        declaration.important,
        specificity,
        shadow_depth,
        order,
        layer,
        attached,
    );
}

/// Record one declared value for `name`, with the cascade facts of the element it was declared
/// on. Takes the property and value by reference: a pre-expanded shorthand hands over a dozen of
/// these, and building a `CssDeclaration` for each only to copy it out again is a dozen
/// allocations per element.
#[allow(clippy::too_many_arguments, reason = "one cascade fact per argument")]
fn push_declaration(
    css_map_entry: &mut CssProperties,
    id: PropertyId,
    value: &Arc<CssValue>,
    sheet: &crate::stylesheet::CssStylesheet,
    important: bool,
    specificity: Specificity,
    shadow_depth: u16,
    order: u32,
    layer: Option<u32>,
    attached: bool,
) {
    let declaration = DeclarationProperty {
        // Shared with the rule this came from: a refcount bump per element rather than a
        // deep copy of the value into every map the rule reaches.
        value: Arc::clone(value),
        origin: sheet.origin,
        important,
        location: Arc::clone(&sheet.url),
        specificity,
        shadow_depth,
        order,
        layer,
        attached,
    };

    css_map_entry.entry(id).declared.push(declaration);
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
        // Only *collapsible* whitespace makes a text node unrenderable. `char::is_whitespace` is
        // the Unicode set, which includes U+00A0 NO-BREAK SPACE and the other fixed-width spaces -
        // characters CSS renders like any other, and which exist precisely to be kept. Parsoid
        // wraps every entity in its own element, so `Designed<span>&nbsp;</span>by` had the whole
        // span dropped and Wikipedia's infoboxes read "Designedby", "Firstappeared", "May1, 1964".
        NodeType::TextNode => doc
            .text_value(id)
            .is_some_and(|v| v.chars().all(|c: char| c.is_ascii_whitespace())),
        _ => false,
    }
}

/// Resolve every substitution function in a declaration's value against this element.
///
/// css-variables-1 §3 substitutes a `var()` on the token stream *before* the value is parsed, so
/// a reference anywhere in the value - including inside another function's arguments - is
/// replaced by the custom property's tokens and the result is then read as if the author had
/// written it that way. `attr()` (css-values-5 §12.1) and `light-dark()` substitute the same way.
///
/// An empty result is the guaranteed-invalid value: the declaration matches no grammar and is
/// dropped by the caller.
pub fn resolve_functions<C: HasDocument>(
    value: &CssValue,
    doc: &C::Document,
    id: NodeId,
    custom_props: &HashMap<String, CssValue>,
) -> CssValue {
    resolve_substitutions(value, custom_props, &|args| resolve_attr::<C>(args, doc, id))
}

/// How an `attr()` is answered: what its arguments stand for on this element. Taking it as a
/// callback keeps the substitution walk itself free of the document, so what a value substitutes
/// to can be asked without building one.
type AttrResolver<'a> = &'a dyn Fn(&[CssValue]) -> Vec<CssValue>;

fn resolve_substitutions(
    value: &CssValue,
    custom_props: &HashMap<String, CssValue>,
    attr: AttrResolver<'_>,
) -> CssValue {
    match value {
        // Only a function or a list can hold a reference; a plain token substitutes to itself,
        // and is handed back in the shape it arrived in.
        CssValue::Function(..) | CssValue::List(_) => {
            CssValue::List(resolve_value(value, custom_props, attr, 0).unwrap_or_default())
        }
        other => other.clone(),
    }
}

/// The tokens `value` substitutes to, or `None` when a substitution function in it is invalid at
/// computed-value time - which makes the whole declaration invalid (css-variables-1 §3.1).
///
/// A result is a list of tokens rather than a single value because a `var()` may stand for
/// several (`--rule: 1px solid red`). They are spliced into the surrounding list or argument
/// list rather than nested as a sub-list: the grammar matcher reads a flat sequence, so
/// `border: 1px solid var(--rule)` has to arrive as the five tokens it would have been written
/// as, not as three with a list in the middle.
///
/// `depth` bounds how many rounds of substitution feed each other - a custom property holding an
/// `attr()` whose fallback holds a `var()`, and so on - and shares its bound with the chain of
/// custom-property references [`resolve_var`] follows.
fn resolve_value(
    value: &CssValue,
    custom_props: &HashMap<String, CssValue>,
    attr: AttrResolver<'_>,
    depth: usize,
) -> Option<Vec<CssValue>> {
    match value {
        CssValue::List(list) => resolve_list(list, custom_props, attr, depth),
        CssValue::Function(name, args) => {
            if depth >= MAX_VAR_DEPTH {
                return None;
            }
            let substituted = if name.eq_ignore_ascii_case("var") {
                Some(resolve_var(args, custom_props))
            } else if name.eq_ignore_ascii_case("attr") {
                Some(attr(args))
            } else if name.eq_ignore_ascii_case("light-dark") || name.eq_ignore_ascii_case("-internal-light-dark") {
                // Unresolved, the whole declaration fails validation - the UA sheet uses it on
                // form controls.
                Some(
                    args.split(|v| matches!(v, CssValue::Comma))
                        .nth(usize::from(crate::stylesheet::prefers_dark()))
                        .map_or_else(Vec::new, <[CssValue]>::to_vec),
                )
            } else {
                None
            };

            match substituted {
                // Nothing to substitute: the reference is undefined with no usable fallback, or
                // cyclic. The declaration is invalid at computed-value time.
                Some(tokens) if tokens.is_empty() => None,
                // What one substitution produced may itself hold another - a custom property
                // holding `attr(data-w px)`, an `attr()` fallback holding a `var()`.
                Some(tokens) => resolve_list(&tokens, custom_props, attr, depth + 1),
                // Any other function keeps its own meaning and is rebuilt around its substituted
                // arguments, so that `rgb(var(--r) 0 0)` and `calc(var(--w) * 2)` reach the
                // grammar as the colour and the length they name.
                //
                // `min`/`max`/`clamp` are still not *evaluated* against this element. Their
                // operands may be font-relative, and this runs while declarations are still being
                // collected - before the element's font-size is known - so an `em` would be
                // measured against the default 16px. `min(2em, 50px)` came out as 32px on an
                // element with `font-size: 20px`, where it should be 40px. The computed stage
                // evaluates them instead, once the basis exists; `reduce_function` reduces only
                // what the parser itself could have, knowing no more than it did.
                None => {
                    let args = resolve_list(args, custom_props, attr, depth)?;
                    Some(vec![reduce_function(name.clone(), args)])
                }
            }
        }
        other => Some(vec![other.clone()]),
    }
}

/// Resolve every value in a sequence, splicing what each one substitutes to into one flat list.
fn resolve_list(
    values: &[CssValue],
    custom_props: &HashMap<String, CssValue>,
    attr: AttrResolver<'_>,
    depth: usize,
) -> Option<Vec<CssValue>> {
    let mut resolved = Vec::with_capacity(values.len());
    for value in values {
        resolved.extend(resolve_value(value, custom_props, attr, depth)?);
    }
    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::RgbColor;

    /// The value of the one declaration in `a { <declaration> }`, as the parser produces it - so
    /// a test is written against the text an author types and reads the shapes a stylesheet
    /// really carries.
    fn declared(declaration: &str) -> CssValue {
        let sheet = Css3::parse_str(
            &format!("a {{ {declaration} }}"),
            ParserConfig {
                ignore_errors: true,
                ..Default::default()
            },
            CssOrigin::Author,
            "",
        )
        .expect("the test declaration parses");
        (*sheet.rules[0].declarations()[0].value).clone()
    }

    /// The custom properties `pairs` declares, each written as it would be in a rule.
    fn custom(pairs: &[(&str, &str)]) -> HashMap<String, CssValue> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), declared(&format!("{name}: {value}"))))
            .collect()
    }

    /// `attr()` as a document with `data-w="4"` would answer it, which is all these tests need
    /// of one: the walk is what is under test, not the reading of an attribute.
    fn attr(args: &[CssValue]) -> Vec<CssValue> {
        match args.first() {
            Some(CssValue::String(name)) if name == "data-w" => vec![CssValue::Unit(4.0, "px".to_string())],
            _ => vec![],
        }
    }

    fn resolve(declaration: &str, props: &[(&str, &str)]) -> CssValue {
        single_value(resolve_substitutions(&declared(declaration), &custom(props), &attr))
    }

    #[test]
    fn a_var_inside_a_colour_function_makes_a_colour() {
        // The substituted value is read as if it had been written that way (css-variables-1 §3),
        // so what comes back is the colour, not a function the `<color>` grammar cannot match.
        let value = resolve("color: rgb(var(--r) 0 0)", &[("--r", "59")]);
        assert_eq!(value, CssValue::Color(RgbColor::from("#3b0000").into()));
    }

    #[test]
    fn a_var_inside_a_calc_is_folded() {
        // `calc()` reaches here as a call with its body as arguments, so the substitution goes
        // into the body and the arithmetic is done on the way out.
        let value = resolve("width: calc(var(--w) * 2)", &[("--w", "10px")]);
        assert_eq!(value, CssValue::Function("calc".to_string(), vec![unit(20.0, "px")]));
    }

    #[test]
    fn a_var_two_functions_deep_is_substituted() {
        let value = resolve(
            "background: linear-gradient(rgb(var(--r) 0 0), white)",
            &[("--r", "59")],
        );
        let CssValue::Function(name, args) = value else {
            panic!("expected the gradient to survive as a function");
        };
        assert_eq!(name, "linear-gradient");
        assert_eq!(args[0], CssValue::Color(RgbColor::from("#3b0000").into()));
    }

    #[test]
    fn a_fallback_inside_a_function_is_used() {
        let value = resolve("width: calc(var(--missing, 10px) * 2)", &[]);
        assert_eq!(value, CssValue::Function("calc".to_string(), vec![unit(20.0, "px")]));
    }

    #[test]
    fn an_unresolvable_var_inside_a_function_invalidates_the_declaration() {
        // No fallback and nothing to substitute is the guaranteed-invalid value, and that makes
        // the whole declaration invalid at computed-value time (css-variables-1 §3.1) - not just
        // the function it sits in.
        assert_eq!(resolve("color: rgb(var(--nope) 0 0)", &[]), CssValue::List(vec![]));
    }

    #[test]
    fn a_cycle_inside_a_function_terminates() {
        let props = custom(&[("--a", "var(--b)"), ("--b", "var(--a)")]);
        let value = resolve_substitutions(&declared("color: rgb(var(--a) 0 0)"), &props, &attr);
        assert_eq!(value, CssValue::List(vec![]));
    }

    #[test]
    fn an_attr_inside_a_function_is_substituted() {
        let value = resolve("width: calc(attr(data-w px) * 2)", &[]);
        assert_eq!(value, CssValue::Function("calc".to_string(), vec![unit(8.0, "px")]));
    }

    #[test]
    fn a_var_holding_several_tokens_splices_into_an_argument_list() {
        // The tokens go into the argument list flat: a nested list would be a single argument,
        // which is not what `rgb(59 130 246)` is.
        let value = resolve("color: rgb(var(--channels))", &[("--channels", "59 130 246")]);
        assert_eq!(value, CssValue::Color(RgbColor::from("#3b82f6").into()));
    }

    fn unit(value: f64, unit: &str) -> CssValue {
        CssValue::Unit(value, unit.to_string())
    }
}
