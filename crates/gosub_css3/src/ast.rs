use cow_utils::CowUtils;
use log::warn;

use crate::media_query::MediaQueryList;
use crate::node::{Node as CssNode, NodeType};
use crate::stylesheet::{
    AttributeSelector, Combinator, CssDeclaration, CssRule, CssSelector, CssSelectorPart, CssStylesheet, CssValue,
    FontFace, ImportRule, MatcherType,
};
use crate::supports::SupportsCondition;
use gosub_interface::css3::CssOrigin;
use gosub_shared::errors::{CssError, CssResult};
use std::sync::Arc;

/*

Given the following css:

    * { color: red; }
    h1 { color: blue; }
    h3, h4 { color: rebeccapurple; }
    ul > li { color: green; }

this will parse to an AST, which this function turns into the following structure:

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

/// The four pseudo-elements CSS2 allowed to be written with a single colon. Selectors Level 4
/// keeps them valid for compatibility; every other `:name` is a pseudo-class.
fn is_legacy_pseudo_element(name: &str) -> bool {
    name.eq_ignore_ascii_case("before")
        || name.eq_ignore_ascii_case("after")
        || name.eq_ignore_ascii_case("first-line")
        || name.eq_ignore_ascii_case("first-letter")
}

/// Whether the `[`/`]` in `nodes` pair up into line-name lists whose every entry is an
/// identifier css-grid allows (css-grid-2 §7.2: any `<custom-ident>` except `span` and `auto`;
/// a `<custom-ident>` itself excludes the CSS-wide keywords and `default`). Function arguments
/// are checked the same way, each as its own list.
fn line_names_are_valid(nodes: &[CssNode]) -> bool {
    let mut open = false;
    for node in nodes {
        match &node.node_type {
            NodeType::Operator { value, .. } if value == "[" => {
                if open {
                    return false;
                }
                open = true;
            }
            NodeType::Operator { value, .. } if value == "]" => {
                if !open {
                    return false;
                }
                open = false;
            }
            NodeType::Ident { value } if open => {
                let excluded = [
                    "span",
                    "auto",
                    "default",
                    "initial",
                    "inherit",
                    "unset",
                    "revert",
                    "revert-layer",
                ];
                if excluded.iter().any(|k| value.eq_ignore_ascii_case(k)) {
                    return false;
                }
            }
            _ if open => return false,
            NodeType::Function { arguments, .. } if !line_names_are_valid(arguments) => return false,
            _ => {}
        }
    }
    !open
}

/// Whether every math expression in this value node spaces its `+` and `-` the way
/// css-values-4 §10.1 requires: whitespace on *both* sides.
///
/// The rule exists because the sign is otherwise part of the number - `calc(1px -2px)` is two
/// adjacent values, not a subtraction - and a UA that guessed would accept CSS no other UA does.
///
/// `in_math` tracks whether we are inside a math function's arguments, because the rule applies
/// only there: the `+` in `rgb(1 2 3 / +0.5)` is not arithmetic.
///
/// A `calc()` body is its own node type rather than a function with arguments, so it needs its
/// own arm - without one, `calc(1px+ 2px)` was folded to `3px` while the same expression inside
/// `min()` was rejected.
fn math_spacing_is_valid(node: &CssNode, in_math: bool) -> bool {
    match &node.node_type {
        NodeType::Operator {
            value,
            space_before,
            space_after,
        } if in_math && (value == "+" || value == "-") => *space_before && *space_after,
        NodeType::Function { name, arguments } => {
            // A math function nested anywhere still has to obey the rule, so the flag only ever
            // turns on as we descend.
            let inside = in_math || crate::functions::calc::is_math_function_name(name);
            arguments.iter().all(|arg| math_spacing_is_valid(arg, inside))
        }
        // `calc()` is parsed by a path of its own and keeps its body as a flat token list, so it
        // is not a `Function` and has to be descended into separately. Everything in there is
        // arithmetic by definition.
        NodeType::Calc { tokens } => tokens.iter().all(|token| math_spacing_is_valid(token, true)),
        _ => true,
    }
}

/// Convert a functional pseudo-class's selector-list argument (as `:not()` takes) into one
/// compound per comma-separated selector.
fn convert_selector_list(arguments: Vec<CssNode>) -> CssResult<Vec<Vec<CssSelectorPart>>> {
    let mut out: Vec<Vec<CssSelectorPart>> = vec![vec![]];
    for argument in arguments {
        let selectors = match argument.node_type {
            NodeType::SelectorList { selectors } => selectors,
            // A single selector with no comma parses as a bare `Selector`.
            NodeType::Selector { children } => {
                convert_selector_children(children, &mut out)?;
                continue;
            }
            _ => continue,
        };
        for selector in selectors {
            if let NodeType::Selector { children } = selector.node_type {
                convert_selector_children(children, &mut out)?;
            }
        }
    }

    out.retain(|compound| !compound.is_empty());
    Ok(out)
}

/// Convert the children of one `Selector` AST node into selector parts, appending to the compound
/// currently being built in `out`. A comma starts a new compound.
fn convert_selector_children(children: Vec<CssNode>, out: &mut Vec<Vec<CssSelectorPart>>) -> CssResult<()> {
    for node in children {
        let part = match node.node_type {
            NodeType::Ident { value } => CssSelectorPart::Type(value),
            NodeType::ClassSelector { value } => CssSelectorPart::Class(value),
            NodeType::Combinator { value } => {
                let combinator = match value.as_str() {
                    ">" => Combinator::Child,
                    "+" => Combinator::NextSibling,
                    "~" => Combinator::SubsequentSibling,
                    " " => Combinator::Descendant,
                    "||" => Combinator::Column,
                    "|" => Combinator::Namespace,
                    _ => return Err(CssError::new(format!("Unknown combinator: {value}").as_str())),
                };

                CssSelectorPart::Combinator(combinator)
            }
            NodeType::IdSelector { value } => CssSelectorPart::Id(value),
            NodeType::TypeSelector { value, .. } if value == "*" => CssSelectorPart::Universal,
            // CSS2 spelled the pseudo-*elements* with a single colon, and that is still
            // what most older stylesheets use (`.container:after` for the clearfix
            // idiom). The tokenizer can only see one colon and reports a pseudo-class,
            // so re-classify the four legacy names here - matching them as pseudo-classes
            // would silently generate no box at all.
            NodeType::PseudoClassSelector { value, .. } => {
                // `:not()` carries a real selector list, which the parser has already built. Keep
                // it structured instead of flattening it to the string ":not(.foo)": matched as an
                // opaque name it can never be evaluated, and the whole rule silently applies to
                // nothing.
                if let NodeType::Function { name, arguments } = value.node_type {
                    if name.eq_ignore_ascii_case("not") {
                        CssSelectorPart::Not(convert_selector_list(arguments)?)
                    } else if name.eq_ignore_ascii_case("host") {
                        // `:host(<selector>)` - the condition is matched against the host
                        // element itself, so it has to stay structured like `:not()`'s.
                        CssSelectorPart::Host(Some(convert_selector_list(arguments)?))
                    } else {
                        // Any other functional pseudo-class keeps its serialized form, which is
                        // what the matcher's name-based arms expect.
                        CssSelectorPart::PseudoClass(
                            CssNode::new(NodeType::Function { name, arguments }, node.location).to_string(),
                        )
                    }
                } else {
                    let name = value.to_string();
                    if is_legacy_pseudo_element(&name) {
                        CssSelectorPart::PseudoElement(name)
                    } else if name.eq_ignore_ascii_case("host") {
                        CssSelectorPart::Host(None)
                    } else {
                        CssSelectorPart::PseudoClass(name)
                    }
                }
            }
            NodeType::PseudoElementSelector { value, arguments } => {
                // `::slotted(<selector>)` keeps its argument; every other functional
                // pseudo-element is still matched by name alone.
                match arguments {
                    Some(args) if value.eq_ignore_ascii_case("slotted") => {
                        CssSelectorPart::Slotted(convert_selector_list(vec![*args])?)
                    }
                    _ => CssSelectorPart::PseudoElement(value),
                }
            }
            NodeType::TypeSelector { value, .. } => CssSelectorPart::Type(value),
            NodeType::AttributeSelector {
                name,
                value,
                flags,
                matcher,
            } => {
                let matcher = match matcher {
                    None => MatcherType::None,

                    Some(matcher) => {
                        if let NodeType::Operator { value: op, .. } = &matcher.node_type {
                            match op.as_str() {
                                "=" => MatcherType::Equals,
                                "~=" => MatcherType::Includes,
                                "|=" => MatcherType::DashMatch,
                                "^=" => MatcherType::PrefixMatch,
                                "$=" => MatcherType::SuffixMatch,
                                "*=" => MatcherType::SubstringMatch,
                                _ => {
                                    warn!("Unsupported matcher: {matcher:?}");
                                    MatcherType::Equals
                                }
                            }
                        } else {
                            warn!("Unsupported matcher: {matcher:?}");
                            MatcherType::Equals
                        }
                    }
                };

                CssSelectorPart::Attribute(Box::new(AttributeSelector {
                    name,
                    matcher,
                    value,
                    case_insensitive: flags.eq_ignore_ascii_case("i"),
                }))
            }
            NodeType::Comma => {
                out.push(vec![]);
                continue;
            }
            other => {
                // Both the text and the shape: the text says which selector an author has to
                // look at, the shape which node type the converter is missing.
                let part = CssNode::new(other, node.location);
                return Err(CssError::new(
                    format!("Unsupported selector part: {part} ({:?})", part.node_type).as_str(),
                ));
            }
        };
        if let Some(x) = out.last_mut() {
            x.push(part);
        } else {
            out.push(vec![part]); //unreachable, but still, we handle it
        }
    }

    Ok(())
}

/// Build one style rule from its prelude and block, or `None` when the rule is invalid.
///
/// Every way this returns `None` is a rule that contributes nothing and nothing more: an invalid
/// selector list invalidates the style rule it belongs to and leaves the rest of the sheet
/// standing (css-syntax-3 §9, selectors-4 §3.9). It used to abandon the whole stylesheet, so a
/// single selector this converter has no arm for - a bare number, dimension or percentage in a
/// compound, or the nesting selector `&` - cost a page every rule its sheet had.
fn collect_rule(
    prelude: Option<Box<CssNode>>,
    block: Option<Box<CssNode>>,
    media: &[Arc<MediaQueryList>],
    layer: Option<u32>,
) -> Option<CssRule> {
    let mut rule = CssRule::new(vec![], vec![], (!media.is_empty()).then(|| media.to_vec()), layer);

    if let Some(node) = prelude {
        let NodeType::SelectorList { selectors } = node.node_type else {
            return None;
        };

        let mut parts: Vec<Vec<CssSelectorPart>> = vec![vec![]];
        for node in selectors {
            let NodeType::Selector { children } = node.node_type else {
                continue;
            };

            if let Err(err) = convert_selector_children(children, &mut parts) {
                // One member of a selector list being invalid invalidates the whole list, and so
                // this rule - but only this rule.
                log::debug!("Rule dropped: {err}");
                return None;
            }
        }

        // A compound with no parts matches every element vacuously, so an empty prelude
        // (e.g. `/*.a, .b*/{ ... }`, where the whole selector list is commented out) would
        // apply its declarations to the entire document. Per CSS Syntax a style rule with an
        // invalid or empty prelude is invalid and must be dropped, so drop the empty compounds
        // and the rule with them if nothing is left.
        parts.retain(|part| !part.is_empty());
        if parts.is_empty() {
            return None;
        }

        rule.selectors.push(CssSelector::new(parts));
    }

    if let Some(declaration) = block {
        let NodeType::Block { children } = declaration.node_type else {
            return None;
        };
        for declaration in children {
            let NodeType::Declaration {
                property,
                value,
                important,
            } = declaration.node_type
            else {
                continue;
            };

            // A math expression with an ill-spaced `+` or `-` makes the whole declaration
            // invalid, so it is checked before any value is converted. Dropping only the
            // offending value would leave `margin: 1px min(1px+ 2px, 9px)` behind as
            // `margin: 1px`, which is not what the author wrote and not what the cascade
            // should see.
            if value.iter().any(|node| !math_spacing_is_valid(node, false)) {
                continue;
            }
            // Likewise a bracketed line-name list that is not one: brackets that do not pair
            // up (`random-item(auto, ])`), or a name css-grid excludes (`[auto]`, `[span]`).
            if !line_names_are_valid(&value) {
                continue;
            }

            // Convert the nodes into CSS Values. A component value that does not convert makes
            // the declaration invalid (css-syntax-3 §9), so it is dropped whole: keeping the
            // tokens that did convert would leave `margin: 1px <unconvertible>` behind as
            // `margin: 1px`, which is not what the author wrote.
            let mut css_values = Vec::with_capacity(value.len());
            for node in value {
                match CssValue::parse_ast_node(node) {
                    Ok(value) => css_values.push(value),
                    Err(err) => {
                        log::debug!("Declaration dropped, {property}: {err}");
                        css_values.clear();
                        break;
                    }
                }
            }

            if css_values.is_empty() {
                continue;
            }

            let value = match css_values.pop() {
                Some(value) if css_values.is_empty() => value,
                Some(value) => {
                    css_values.push(value);
                    CssValue::List(css_values)
                }
                None => CssValue::List(css_values),
            };

            // The four prefixed `display` keywords the Compatibility Standard requires are
            // the standard values they alias. Resolved here so that every reader of a
            // stylesheet - the cascade and the CSSOM alike - sees the value they mean.
            let value = resolve_display_alias(&property, value);

            rule.declarations.push(CssDeclaration {
                // Resolved to an id here, once, rather than by name on every rule expansion
                // and every element a pending declaration reaches.
                property: property.into(),
                value,
                important,
            });
        }
    }

    Some(rule)
}

/// The prefixed `display` values the Compatibility Standard requires every engine to support,
/// and the standard value each one means.
///
/// This is the whole of the vendor-prefixed *value* support: a prefixed keyword any other
/// property is given is a value this engine does not implement, so the declaration is invalid
/// and the cascade falls back to the standard declaration beside it.
///
/// This used to be a blanket rule that stripped a known prefix from every string value of every
/// declaration. That accepted things no engine supports (`cursor: -webkit-grab` became `grab`)
/// and, worse, rewrote author-chosen names that merely start with a dash - an
/// `animation-name: -webkit-spin` ran a different animation than the one the page defined.
const DISPLAY_ALIASES: [(&str, &str); 4] = [
    ("-webkit-box", "flex"),
    ("-webkit-inline-box", "inline-flex"),
    ("-webkit-flex", "flex"),
    ("-webkit-inline-flex", "inline-flex"),
];

/// Resolve a prefixed `display` keyword to the value it aliases, leaving everything else alone.
pub(crate) fn resolve_display_alias(property: &str, value: CssValue) -> CssValue {
    if !property.eq_ignore_ascii_case("display") {
        return value;
    }
    let CssValue::String(keyword) = &value else {
        return value;
    };
    match DISPLAY_ALIASES
        .iter()
        .find(|(alias, _)| keyword.eq_ignore_ascii_case(alias))
    {
        Some((_, standard)) => CssValue::String((*standard).to_string()),
        None => value,
    }
}

/// Build an [`ImportRule`] from an `@import` prelude ([`NodeType::ImportList`]).
///
/// The children arrive in grammar order: the target, then an optional layer, an optional
/// `supports()`, and an optional media query list. Returns `None` when no target is present.
fn collect_import(prelude: &CssNode) -> Option<ImportRule> {
    let NodeType::ImportList { children } = &prelude.node_type else {
        return None;
    };

    let mut url = None;
    let mut layer = None;
    let mut supports = None;
    let mut media = None;

    for child in children {
        match &child.node_type {
            NodeType::String { value } if url.is_none() => url = Some(value.clone()),
            NodeType::Url { url: value } if url.is_none() => url = Some(value.clone()),
            NodeType::Ident { value } if value.eq_ignore_ascii_case("layer") => layer = Some(None),
            NodeType::Function { name, arguments } if name.eq_ignore_ascii_case("layer") => {
                let named = arguments.iter().find_map(|arg| match &arg.node_type {
                    NodeType::Ident { value } => Some(value.clone()),
                    _ => None,
                });
                layer = Some(named);
            }
            // The parser hands the `supports(...)` interior back as raw text.
            NodeType::Raw { value } => supports = Some(SupportsCondition::parse_import_condition(value)),
            NodeType::MediaQueryList { .. } => media = Some(MediaQueryList::from_ast(child)),
            _ => {}
        }
    }

    Some(ImportRule {
        url: url?,
        layer,
        supports,
        media,
    })
}

/// The layer names in an `@layer` prelude, in the order they were written.
fn layer_names(prelude: &CssNode) -> Vec<String> {
    let NodeType::LayerList { layers } = &prelude.node_type else {
        return Vec::new();
    };
    layers
        .iter()
        .filter_map(|node| match &node.node_type {
            NodeType::Ident { value } => Some(value.clone()),
            _ => None,
        })
        .collect()
}

/// A layer's full name: the one written, under the layer it was written inside.
fn qualify_layer(outer: Option<&str>, name: &str) -> String {
    match outer {
        Some(outer) => format!("{outer}.{name}"),
        None => name.to_string(),
    }
}

/// Record a layer under its full name, returning its index. A name already declared keeps the
/// place it first had: reopening a layer adds to it, it does not move it (css-cascade-5 §6.4.2).
///
/// Declaring `a.b` declares `a` as well, because `a` has to have a place of its own for `a.b` to
/// sort inside it.
fn register_layer(layers: &mut Vec<String>, name: &str) -> u32 {
    let mut path = String::new();
    let mut index = 0;
    for part in name.split('.') {
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(part);
        index = match layers.iter().position(|known| *known == path) {
            Some(known) => known,
            None => {
                layers.push(path.clone());
                layers.len() - 1
            }
        };
    }
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Walk a stylesheet's top-level nodes, flattening at-rules into a single rule list.
///
/// `media` is the stack of `@media` conditions currently in scope, outermost first; every rule
/// collected while it is non-empty records it and is evaluated against the live
/// [`MediaEnvironment`](crate::media_query::MediaEnvironment) at match time rather than here.
fn collect_rules(
    nodes: Vec<CssNode>,
    rules: &mut Vec<CssRule>,
    font_faces: &mut Vec<FontFace>,
    imports: &mut Vec<ImportRule>,
    media: &mut Vec<Arc<MediaQueryList>>,
    layers: &mut Vec<String>,
    layer: Option<u32>,
) {
    for node in nodes {
        match node.node_type {
            NodeType::Rule { prelude, block } => {
                if let Some(rule) = collect_rule(prelude, block, media, layer) {
                    rules.push(rule);
                }
            }
            NodeType::AtRule {
                name,
                prelude,
                block: Some(block),
            } if name.eq_ignore_ascii_case("media") => {
                if let NodeType::Block { children } = block.node_type {
                    // A missing or unparseable prelude yields an empty (always-matching) list,
                    // so the block's rules stay visible rather than disappearing.
                    let list = prelude.map(|node| MediaQueryList::from_ast(&node)).unwrap_or_default();
                    media.push(Arc::new(list));
                    collect_rules(children, rules, font_faces, imports, media, layers, layer);
                    media.pop();
                }
            }
            NodeType::AtRule {
                name,
                prelude: Some(prelude),
                block: None,
            } if name.eq_ignore_ascii_case("import") => {
                // Per spec `@import` may only appear before any style rule; a later one is
                // invalid and ignored. Enforcing that keeps `splice_import`'s "imported rules
                // go in front" contract honest.
                if rules.is_empty() {
                    if let Some(import) = collect_import(&prelude) {
                        imports.push(import);
                    }
                } else {
                    warn!("Ignoring @import that follows a style rule");
                }
            }
            NodeType::AtRule {
                name,
                prelude,
                block: Some(block),
            } if name.eq_ignore_ascii_case("supports") => {
                // A supports condition asks about the engine, never the device, so it can be
                // settled here: a false block contributes no rules at all, and a true one
                // flattens away exactly like `@layer`.
                let holds = match prelude.as_deref() {
                    Some(CssNode {
                        node_type: NodeType::Raw { value },
                        ..
                    }) => SupportsCondition::parse(value).matches(),
                    // No prelude at all is not a valid `@supports`; drop the block.
                    _ => false,
                };
                if holds {
                    if let NodeType::Block { children } = block.node_type {
                        collect_rules(children, rules, font_faces, imports, media, layers, layer);
                    }
                }
            }
            // `@layer name { ... }`, or an anonymous `@layer { ... }`. The block's rules are
            // flattened into the sheet like any other conditional group, but they carry the
            // layer with them: which layer a rule sits in decides the cascade before specificity
            // is ever looked at (css-cascade-5 §6.4.1).
            NodeType::AtRule {
                name,
                prelude,
                block: Some(block),
            } if name.eq_ignore_ascii_case("layer") => {
                // A nested `@layer b` inside `@layer a` is the layer `a.b`.
                let outer = layer.and_then(|index| layers.get(index as usize)).cloned();
                let declared = prelude.as_deref().map_or_else(Vec::new, layer_names);
                // At most one name may be given when there is a block; an anonymous layer is
                // its own layer every time, and cannot be reopened, so it gets a name no
                // author-written `@layer` can collide with.
                let name = match declared.first() {
                    Some(name) => qualify_layer(outer.as_deref(), name),
                    None => format!("%anonymous-{}", layers.len()),
                };
                let inner = register_layer(layers, &name);
                if let NodeType::Block { children } = block.node_type {
                    collect_rules(children, rules, font_faces, imports, media, layers, Some(inner));
                }
            }
            // `@layer a, b;` sets the order of layers before either is filled in. It carries no
            // rules; naming them here is its whole purpose.
            NodeType::AtRule {
                name,
                prelude,
                block: None,
            } if name.eq_ignore_ascii_case("layer") => {
                let outer = layer.and_then(|index| layers.get(index as usize)).cloned();
                for declared in prelude.as_deref().map_or_else(Vec::new, layer_names) {
                    register_layer(layers, &qualify_layer(outer.as_deref(), &declared));
                }
            }
            NodeType::AtRule {
                name,
                block: Some(block),
                ..
            } if name.eq_ignore_ascii_case("font-face") => {
                if let NodeType::Block { children } = block.node_type {
                    if let Some(face) = collect_font_face(children) {
                        font_faces.push(face);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Build a [`FontFace`] from the declarations inside an `@font-face` block. Requires a
/// `font-family` and at least one `src: url(...)`; returns `None` otherwise.
fn collect_font_face(nodes: Vec<CssNode>) -> Option<FontFace> {
    let mut family: Option<String> = None;
    let mut sources: Vec<String> = Vec::new();
    let mut unicode_range: Option<String> = None;

    for decl in nodes {
        let NodeType::Declaration {
            property,
            value: value_nodes,
            ..
        } = decl.node_type
        else {
            continue;
        };
        match property.cow_to_ascii_lowercase().as_ref() {
            "font-family" => {
                let name: String = value_nodes
                    .into_iter()
                    .filter_map(|n| CssValue::parse_ast_node(n).ok())
                    .filter_map(|v| match v {
                        CssValue::String(s) => Some(s),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                let name = name.trim().trim_matches(['"', '\'']).trim().to_string();
                if !name.is_empty() {
                    family = Some(name);
                }
            }
            "src" => {
                // `src` is a descriptor, so a later declaration replaces an earlier one rather
                // than adding to it. The "bulletproof @font-face" idiom depends on that: it puts
                // a bare `src: url(...eot)` first for IE<9 and a full `src:` list after it for
                // everyone else. Appending instead of replacing leaves the IE-only EOT at the
                // head of the list, where it is fetched and rejected before any usable format.
                let mut entries: Vec<(String, Option<String>)> = Vec::new();
                for n in value_nodes {
                    if let Ok(v) = CssValue::parse_ast_node(n) {
                        collect_src_entries(&v, &mut entries);
                    }
                }
                sources = entries
                    .into_iter()
                    .filter(|(_, format)| format.as_deref().is_none_or(font_format_is_usable))
                    .map(|(url, _)| url)
                    .collect();
            }
            "unicode-range" => {
                // Reconstruct the raw range list; consumers scan it for `U+xxxx` tokens, so
                // the exact separator/spacing does not matter.
                let raw: String = value_nodes
                    .into_iter()
                    .filter_map(|n| CssValue::parse_ast_node(n).ok())
                    .filter_map(|v| match v {
                        CssValue::String(s) => Some(s),
                        CssValue::Comma => Some(",".to_string()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if !raw.trim().is_empty() {
                    unicode_range = Some(raw);
                }
            }
            _ => {}
        }
    }

    let family = family?;
    if sources.is_empty() {
        return None;
    }
    Some(FontFace {
        family,
        sources,
        unicode_range,
    })
}

/// Whether a `format()` hint names something the font backends can actually decode.
///
/// Only the two formats no backend here reads are rejected: `embedded-opentype` (EOT, an IE-only
/// container) and `svg` (SVG fonts, long dropped from every engine). An unrecognised hint is kept
/// and tried, so a format we have not heard of never costs us a usable face.
fn font_format_is_usable(format: &str) -> bool {
    !matches!(format, "embedded-opentype" | "svg")
}

/// Recursively collect `url(...)` targets from an `@font-face` `src` value, each paired with the
/// `format(...)` hint that follows it, if any.
fn collect_src_entries(value: &CssValue, out: &mut Vec<(String, Option<String>)>) {
    match value {
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("url") => {
            if let Some(url) = args.iter().find_map(|a| match a {
                CssValue::String(s) => Some(s.trim_matches(['"', '\'']).to_string()),
                _ => None,
            }) {
                if !url.is_empty() {
                    out.push((url, None));
                }
            }
        }
        // A `format()` always follows the url it describes, so it belongs to the last one seen.
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("format") => {
            let hint = args.iter().find_map(|a| match a {
                CssValue::String(s) => Some(s.trim_matches(['"', '\'']).cow_to_ascii_lowercase().into_owned()),
                _ => None,
            });
            if let (Some(hint), Some(last)) = (hint, out.last_mut()) {
                last.1 = Some(hint);
            }
        }
        CssValue::List(list) => {
            for item in list {
                collect_src_entries(item, out);
            }
        }
        _ => {}
    }
}

/// Converts a CSS AST to a CSS stylesheet structure.
///
/// Only one thing can fail here, and it is not about the CSS: being handed something that is not
/// a stylesheet at all. Everything the sheet itself can get wrong - a selector this converter
/// has no arm for, a value that does not convert, an at-rule that makes no sense - invalidates
/// the rule, declaration or at-rule it belongs to and nothing else, which is what css-syntax-3 §9
/// requires. A sheet used to be lost whole to any one of them.
/// Fold one top-level node into a sheet being built.
///
/// The streaming counterpart of [`convert_ast_to_stylesheet`]: the parser hands over each rule
/// as it finishes it, the rule's nodes are converted here, and the caller drops them before the
/// next rule is parsed. `rules` accumulates across calls, which is what keeps `@import`'s "only
/// before any style rule" check honest, and so does `layers`. The media stack is per call
/// because it is balanced within one top-level node - a nested `@media` pushes its condition and
/// pops it again before the node is done.
pub(crate) fn convert_node_into(node: CssNode, sheet: &mut CssStylesheet, layers: &mut Vec<String>) {
    collect_rules(
        vec![node],
        &mut sheet.rules,
        &mut sheet.font_faces,
        &mut sheet.imports,
        &mut Vec::new(),
        layers,
        None,
    );
}

/// Whether any declaration in the sheet uses a viewport-relative unit; see
/// [`CssStylesheet::uses_viewport_units`].
pub(crate) fn note_viewport_units(sheet: &mut CssStylesheet) {
    sheet.uses_viewport_units = sheet
        .rules
        .iter()
        .flat_map(|rule| rule.declarations.iter())
        .any(|decl| decl.value.uses_viewport_units());
}

pub fn convert_ast_to_stylesheet(css_ast: CssNode, origin: CssOrigin, url: &str) -> CssResult<CssStylesheet> {
    let NodeType::StyleSheet { children } = css_ast.node_type else {
        return Err(CssError::new("CSS AST must start with a stylesheet node"));
    };

    let mut sheet = CssStylesheet::new(origin, url);

    let mut layers = Vec::new();
    collect_rules(
        children,
        &mut sheet.rules,
        &mut sheet.font_faces,
        &mut sheet.imports,
        &mut Vec::new(),
        &mut layers,
        None,
    );
    sheet.layers = layers;
    // Recorded once here rather than asked per resize: a sheet using `vw`/`vh` must be
    // restyled whenever the viewport changes, while one that does not can keep its cached
    // computed values (see `CssStylesheet::uses_viewport_units`).
    note_viewport_units(&mut sheet);
    Ok(sheet)
}

#[cfg(test)]
mod tests {

    /// The Compatibility Standard requires the four prefixed `display` keywords, and they mean
    /// the standard value they alias. Everything else prefixed is a value this engine does not
    /// implement, so it is left alone here and the grammar rejects it - it must not be quietly
    /// rewritten into the unprefixed keyword, which is what a blanket prefix-stripping rule did:
    /// it turned `animation-name: -webkit-spin` into a reference to a different animation.
    #[test]
    fn only_the_compat_display_keywords_are_aliased() {
        use crate::stylesheet::CssValue;
        let sheet = crate::Css3::parse_str(
            "a { display: -webkit-flex } b { display: -webkit-box } c { cursor: -webkit-grab } d { animation-name: -webkit-spin }",
            gosub_shared::config::ParserConfig { ignore_errors: true, ..Default::default() },
            gosub_interface::css3::CssOrigin::Author,
            "",
        )
        .expect("parses");
        let value_of = |rule: usize| sheet.rules[rule].declarations()[0].value.clone();
        assert_eq!(value_of(0), CssValue::String("flex".into()));
        assert_eq!(value_of(1), CssValue::String("flex".into()));
        assert_eq!(value_of(2), CssValue::String("-webkit-grab".into()));
        assert_eq!(value_of(3), CssValue::String("-webkit-spin".into()));
    }
    use super::*;
    use crate::media_query::MediaEnvironment;
    use crate::stylesheet::Specificity;
    use crate::Css3;
    use gosub_shared::config::ParserConfig;

    /// A selector this converter has no arm for invalidates its own rule and nothing else
    /// (css-syntax-3 §9). It used to fail the conversion of the whole stylesheet, so one
    /// `.p-0.5` - a class name the tokenizer reads as `.p-0` followed by the number `.5` - cost
    /// the page every rule the sheet had.
    #[test]
    fn an_unsupported_selector_drops_only_its_own_rule() {
        let stylesheet = Css3::parse_str(
            "a { color: red } .p-0.5 { color: green } b { color: blue }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 2, "both valid rules survive the invalid one");
        let kept: Vec<String> = stylesheet
            .rules
            .iter()
            .flat_map(|rule| rule.declarations())
            .map(|declaration| declaration.value.to_string())
            .collect();
        assert_eq!(kept, vec!["red", "blue"], "the rules either side of it are untouched");
    }

    /// One invalid member makes the whole selector list invalid, and so the rule it heads -
    /// but no other rule (selectors-4 §3.9). `h1` does not keep the declaration `h2` loses.
    #[test]
    fn an_invalid_member_drops_its_whole_selector_list() {
        let stylesheet = Css3::parse_str(
            "h1, .p-0.5, h2 { color: red } p { color: blue }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1, "the list goes whole, its neighbour stays");
        assert_eq!(
            stylesheet.rules[0].declarations.first().unwrap().value.to_string(),
            "blue"
        );
    }

    /// A value component that does not convert makes its declaration invalid, and only it.
    /// `filter: progid:...` is IE's, and the tokenizer has a node type of its own for it.
    #[test]
    fn a_declaration_whose_value_does_not_convert_drops_only_itself() {
        let stylesheet = Css3::parse_str(
            "a { color: red; filter: progid:DXImageTransform.Microsoft.Alpha(opacity=65); display: block }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1);
        let properties: Vec<&str> = stylesheet.rules[0]
            .declarations()
            .iter()
            .map(|declaration| declaration.property.as_str())
            .collect();
        assert_eq!(properties, vec!["color", "display"]);
    }

    #[test]
    fn rule_with_fully_commented_out_selector_is_dropped() {
        // slashdot.org's classic.css ships `/*.a, .b*/{ ... }`. The empty prelude used to
        // survive as a single empty compound, which matches every element vacuously and
        // applied `height:64px; position:absolute` to the whole document.
        let stylesheet = Css3::parse_str(
            r#"
            /*#editor header .topic, #firehose article header .topic */{ height: 64px; position: absolute; }
            h1 { color: red; }
            "#,
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1, "only the h1 rule survives");
        assert_eq!(
            stylesheet.rules[0].declarations.first().unwrap().property.as_str(),
            "color"
        );
    }

    #[test]
    fn selector_list_keeps_every_compound() {
        let stylesheet = Css3::parse_str(
            "h3, h4, .foo > .bar { color: red; }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1);
        assert_eq!(
            stylesheet.rules[0].selectors.first().unwrap().complex_count(),
            3,
            "dropping empty compounds must not drop real ones"
        );
    }

    #[test]
    fn single_colon_before_after_are_pseudo_elements() {
        // The clearfix idiom `.container:after { clear: both }` depends on this: matched as a
        // pseudo-*class* the rule generates no box, and nothing contains the floats.
        let stylesheet = Css3::parse_str(
            ".a:after { content: \"\" } .b:hover { color: red }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        let parts: Vec<_> = stylesheet.rules[0].selectors[0].complex_at(0).to_vec();
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, CssSelectorPart::PseudoElement(n) if n == "after")),
            "`:after` must become a pseudo-element, got {parts:?}"
        );

        let parts: Vec<_> = stylesheet.rules[1].selectors[0].complex_at(0).to_vec();
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, CssSelectorPart::PseudoClass(n) if n == "hover")),
            "a real pseudo-class must stay one, got {parts:?}"
        );
    }

    #[test]
    fn bulletproof_font_face_drops_the_ie_only_sources() {
        // slashdot's sdicon face, in the "bulletproof @font-face" shape: a bare EOT `src` for
        // IE<9 followed by a full list. The second `src` replaces the first, and the EOT and SVG
        // entries are dropped by their format hints, so the first source tried is one that works.
        let stylesheet = Css3::parse_str(
            r#"
            @font-face {
              font-family: 'sdicon';
              src: url("//example.org/sdicon.eot");
              src: url("//example.org/sdicon.eot#iefix") format("embedded-opentype"),
                   url("//example.org/sdicon.woff") format("woff"),
                   url("//example.org/sdicon.ttf") format("truetype"),
                   url("//example.org/sdicon.svg#sdicon") format("svg");
            }
            "#,
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        let face = &stylesheet.font_faces[0];
        assert_eq!(face.family, "sdicon");
        assert_eq!(
            face.sources,
            vec!["//example.org/sdicon.woff", "//example.org/sdicon.ttf"]
        );
    }

    #[test]
    fn sources_without_a_format_hint_are_kept() {
        // No hint means no reason to reject it, and an unrecognised hint is tried too.
        let stylesheet = Css3::parse_str(
            r#"
            @font-face {
              font-family: 'x';
              src: url("a.woff2") format("woff2"),
                   url("b.ttf"),
                   url("c.bin") format("some-future-format");
            }
            "#,
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.font_faces[0].sources, vec!["a.woff2", "b.ttf", "c.bin"]);
    }

    #[test]
    fn not_keeps_its_argument_as_a_selector() {
        // Flattened to the string ":not(.skip)" this can never be evaluated, and the rule silently
        // matches nothing - which is how slashdot's badge styling disappeared.
        let stylesheet = Css3::parse_str(
            ".box > span:not(.skip) { color: red }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        let parts = stylesheet.rules[0].selectors[0].complex_at(0);
        let Some(CssSelectorPart::Not(inner)) = parts.last() else {
            panic!("expected a Not part, got {parts:?}");
        };
        assert_eq!(inner.len(), 1, "one compound in the argument");
        assert_eq!(inner[0], vec![CssSelectorPart::Class("skip".to_string())]);
    }

    #[test]
    fn not_accepts_a_selector_list() {
        let stylesheet = Css3::parse_str(
            "span:not(.a, .b) { color: red }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        let parts = stylesheet.rules[0].selectors[0].complex_at(0);
        let Some(CssSelectorPart::Not(inner)) = parts.last() else {
            panic!("expected a Not part, got {parts:?}");
        };
        assert_eq!(inner.len(), 2, "one compound per comma-separated argument");
    }

    #[test]
    fn not_contributes_its_most_specific_argument() {
        // Selectors L4 §17: `:not()` adds nothing itself, but its most specific argument counts.
        let stylesheet = Css3::parse_str(
            "b:not(#nope) { color: red } i:not(.c) { color: red } u:not(s) { color: red }",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        let spec = |i: usize| Specificity::from(stylesheet.rules[i].selectors[0].complex_at(0));
        assert_eq!(spec(0), Specificity::new(1, 0, 1), "an id argument counts as an id");
        assert_eq!(spec(1), Specificity::new(0, 1, 1), "a class argument counts as a class");
        assert_eq!(spec(2), Specificity::new(0, 0, 2), "a type argument counts as a type");
    }

    #[test]
    fn font_face_rules_are_collected() {
        let stylesheet = Css3::parse_str(
            r#"
            @font-face {
              font-family: 'Source Serif 4';
              font-style: normal;
              font-weight: 600;
              src: url(https://example.com/ss.ttf) format('truetype');
              unicode-range: U+0000-00FF, U+0131, U+0152-0153;
            }
            h1 { color: red; }
            "#,
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1, "the h1 rule is still collected");
        assert_eq!(stylesheet.font_faces.len(), 1);
        let face = &stylesheet.font_faces[0];
        assert_eq!(face.family, "Source Serif 4");
        assert_eq!(face.sources, vec!["https://example.com/ss.ttf".to_string()]);
        assert!(face.unicode_range.as_deref().unwrap_or("").contains("U+0000"));
    }

    #[test]
    fn layer_rules_are_flattened() {
        let stylesheet = Css3::parse_str(
            r#"
            @layer base {
                h1 { color: red; }
            }
            h2 { color: blue; }
            @layer utilities {
                h3 { font-size: 1em; }
            }
            "#,
            ParserConfig::default(),
            CssOrigin::User,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 3);
        assert_eq!(
            stylesheet.rules[0].selectors[0].complex_at(0)[0],
            CssSelectorPart::Type("h1".into())
        );
        assert_eq!(
            stylesheet.rules[1].selectors[0].complex_at(0)[0],
            CssSelectorPart::Type("h2".into())
        );
        assert_eq!(
            stylesheet.rules[2].selectors[0].complex_at(0)[0],
            CssSelectorPart::Type("h3".into())
        );
    }

    #[test]
    fn layer_ordering_declaration_is_ignored() {
        let stylesheet = Css3::parse_str(
            r#"
            @layer base, utilities;
            h1 { color: red; }
            "#,
            ParserConfig::default(),
            CssOrigin::User,
            "test.css",
        )
        .unwrap();

        assert_eq!(stylesheet.rules.len(), 1);
    }

    /// Parse a one-rule stylesheet and return its declarations as `name: value` strings.
    fn declarations_of(css: &str) -> Vec<String> {
        Css3::parse_str(
            css,
            ParserConfig {
                ignore_errors: true,
                ..Default::default()
            },
            CssOrigin::Author,
            "test.css",
        )
        .expect("stylesheet should parse")
        .rules
        .first()
        .map(|rule| {
            rule.declarations
                .iter()
                .map(|d| format!("{}: {}", d.property, d.value))
                .collect()
        })
        .unwrap_or_default()
    }

    #[test]
    fn math_functions_require_whitespace_around_plus_and_minus() {
        // css-values-4 §10.1: whitespace on *both* sides of `+` and `-`, or the declaration is
        // invalid. One space short is enough.
        assert_eq!(declarations_of("a{width:min(1px+ 2px, 9px)}"), Vec::<String>::new());
        assert_eq!(declarations_of("a{width:max(1px, 2px+ 3px)}"), Vec::<String>::new());
        assert_eq!(
            declarations_of("a{width:clamp(1px, 2px+ 3px, 9px)}"),
            Vec::<String>::new()
        );
        // `calc()` is parsed by a path of its own, so it needs checking in its own right - it
        // used to be exempt by accident and folded this to `calc(3px)`.
        assert_eq!(declarations_of("a{width:calc(1px+ 2px)}"), Vec::<String>::new());
        // Including through a nested group, which is a call with no name.
        assert_eq!(declarations_of("a{width:calc((1px+ 2px) * 2)}"), Vec::<String>::new());

        // Correctly spaced, so it still folds.
        assert_eq!(declarations_of("a{width:min(1px + 2px, 9px)}"), ["width: calc(3px)"]);
        assert_eq!(declarations_of("a{width:calc(1px + 2px)}"), ["width: calc(3px)"]);

        // The other half of the rule is not this stage's to enforce. `calc(1px -2px)` has no
        // operator in it at all - the tokenizer folded the sign into the number, which is
        // exactly why the spec demands the space - so it parses as two juxtaposed values and
        // survives to here. What rejects it is the matcher, which cannot read it as a sum.
        assert_eq!(declarations_of("a{width:calc(1px -2px)}"), ["width: calc(1px -2px)"]);
    }

    #[test]
    fn one_bad_math_expression_drops_the_whole_declaration() {
        // Not just the offending value: `margin: 1px <invalid>` is invalid as a declaration, and
        // leaving `margin: 1px` behind would apply a value the author never wrote.
        assert_eq!(
            declarations_of("a{margin:1px min(1px+ 2px, 9px)}"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_whitespace_rule_is_only_for_math_functions() {
        // The `+` in an alpha value is a sign, not an operator, so no spacing is required.
        assert_eq!(
            declarations_of("a{color:rgb(1 2 3 / +0.5)}"),
            ["color: rgba(1, 2, 3, 0.5)"]
        );
        // And `*` and `/` need no whitespace even inside a math function.
        assert_eq!(declarations_of("a{width:min(1px*2, 9px)}"), ["width: calc(2px)"]);
    }

    #[test]
    fn a_comment_does_not_end_a_value_list() {
        // The whitespace/comment skip used to consume a single token, so a comment that followed
        // whitespace was handed to `parse_value`, which stopped the list there: this came out as
        // `margin: 1px`.
        assert_eq!(declarations_of("a{margin:1px /* c */ 2px}"), ["margin: 1px 2px"]);
    }

    /// Parse `css` and return its collected imports.
    fn imports_of(css: &str) -> Vec<crate::stylesheet::ImportRule> {
        Css3::parse_str(
            css,
            ParserConfig {
                ignore_errors: true,
                ..Default::default()
            },
            CssOrigin::Author,
            "test.css",
        )
        .expect("stylesheet should parse")
        .imports
    }

    #[test]
    fn imports_are_collected_in_every_target_form() {
        let imports = imports_of(
            r#"
            @import "plain.css";
            @import url("quoted.css");
            @import url(bare.css);
            "#,
        );
        let urls: Vec<&str> = imports.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(urls, vec!["plain.css", "quoted.css", "bare.css"]);
    }

    /// The common real-world form. Before the parser learned to read the trailing media query
    /// list, the leftover tokens failed the caller's semicolon check and the whole rule was
    /// discarded.
    #[test]
    fn import_carries_its_media_query_list() {
        let imports = imports_of(r#"@import url("mobile.css") screen and (max-width: 600px);"#);
        assert_eq!(imports.len(), 1, "the import must survive the trailing media query");
        assert_eq!(imports[0].url, "mobile.css");

        let media = imports[0].media.as_ref().expect("media query list recorded");
        let narrow = MediaEnvironment {
            width: 400.0,
            ..Default::default()
        };
        let wide = MediaEnvironment {
            width: 1200.0,
            ..Default::default()
        };
        assert!(media.matches(&narrow));
        assert!(!media.matches(&wide));
    }

    #[test]
    fn import_layer_forms() {
        // The bare keyword used to be peeked at but never consumed, which dropped the rule.
        let imports = imports_of(r#"@import "a.css" layer;"#);
        assert_eq!(imports.len(), 1, "bare `layer` must not drop the import");
        assert_eq!(imports[0].layer, Some(None));

        let imports = imports_of(r#"@import "a.css" layer(base);"#);
        assert_eq!(imports[0].layer, Some(Some("base".to_string())));

        let imports = imports_of(r#"@import "a.css";"#);
        assert_eq!(imports[0].layer, None);
    }

    /// `supports(display: grid)` contains a colon, which `parse_function` rejects - and the
    /// error used to take the whole `@import` with it. The interior is captured raw and run
    /// through the same evaluator `@supports` uses.
    #[test]
    fn import_supports_condition_is_evaluated() {
        let imports = imports_of(r#"@import "a.css" supports(display: grid);"#);
        assert_eq!(imports.len(), 1, "the import must survive its supports() condition");
        assert!(imports[0].supports.as_ref().expect("condition recorded").matches());

        let imports = imports_of(r#"@import "a.css" supports(display: bogus-value);"#);
        assert!(!imports[0].supports.as_ref().expect("condition recorded").matches());
    }

    /// All four optional parts at once, in grammar order.
    #[test]
    fn import_with_every_optional_part() {
        let imports =
            imports_of(r#"@import url("a.css") layer(base) supports(display: grid) screen and (min-width: 40em);"#);
        assert_eq!(imports.len(), 1);
        let import = &imports[0];
        assert_eq!(import.url, "a.css");
        assert_eq!(import.layer, Some(Some("base".to_string())));
        assert!(import.supports.as_ref().expect("supports").matches());
        assert!(import.media.as_ref().expect("media").matches(&MediaEnvironment {
            width: 800.0,
            ..Default::default()
        }));
    }

    /// `@import` is only valid before any style rule; a later one is ignored, which is what
    /// lets imported rules always be spliced in at the front.
    #[test]
    fn import_after_a_style_rule_is_ignored() {
        let imports = imports_of(
            r#"
            @import "first.css";
            h1 { color: red; }
            @import "too-late.css";
            "#,
        );
        let urls: Vec<&str> = imports.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(urls, vec!["first.css"]);
    }

    #[test]
    fn supports_block_is_kept_or_dropped_by_its_condition() {
        // A condition the engine satisfies: the inner rules flatten out, like `@layer`.
        let sheet = Css3::parse_str(
            r"
            @supports (display: grid) {
                h1 { color: red; }
            }
            @supports (display: bogus-value) {
                h2 { color: blue; }
            }
            ",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(sheet.rules.len(), 1, "only the satisfied block contributes rules");
        assert_eq!(
            sheet.rules[0].selectors[0].complex_at(0)[0],
            CssSelectorPart::Type("h1".into())
        );
    }

    /// `@media` inside `@supports` keeps its condition; the supports gate is resolved here and
    /// leaves no trace on the rule.
    #[test]
    fn media_nested_in_supports() {
        let sheet = Css3::parse_str(
            r"
            @supports (display: grid) {
                @media (min-width: 600px) {
                    h1 { color: red; }
                }
            }
            ",
            ParserConfig::default(),
            CssOrigin::Author,
            "test.css",
        )
        .unwrap();

        assert_eq!(sheet.rules.len(), 1);
        let media = sheet.rules[0].media.as_ref().expect("media condition survives");
        assert_eq!(media.len(), 1);
        assert!(media[0].matches(&MediaEnvironment {
            width: 800.0,
            ..Default::default()
        }));
        assert!(!media[0].matches(&MediaEnvironment {
            width: 400.0,
            ..Default::default()
        }));
    }

    #[test]
    fn convert_font_family() {
        let _stylesheet = Css3::parse_str(
            r#"
              body {
                border: 1px solid black;
                color: #ffffff;
                background-color: #121212;
                font-family: "Arial", sans-serif;
                margin: 0;
                padding: 0;
              }
            "#,
            ParserConfig::default(),
            CssOrigin::User,
            "test.css",
        )
        .unwrap();
    }

    #[test]
    fn convert_test() {
        let stylesheet = Css3::parse_str(
            r"
            h1 { color: red; }
            h3, h4 { border: 1px solid black; }
            ",
            ParserConfig::default(),
            CssOrigin::User,
            "test.css",
        )
        .unwrap();

        assert_eq!(
            stylesheet
                .rules
                .first()
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .property
                .as_str(),
            "color"
        );
        assert_eq!(
            stylesheet.rules.first().unwrap().declarations.first().unwrap().value,
            CssValue::String("red".into())
        );

        assert_eq!(
            stylesheet
                .rules
                .get(1)
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .property
                .as_str(),
            "border"
        );
        assert_eq!(
            stylesheet.rules.get(1).unwrap().declarations.first().unwrap().value,
            CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::String("black".into())
            ])
        );
    }
}
