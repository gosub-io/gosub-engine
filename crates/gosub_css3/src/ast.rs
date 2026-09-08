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
                    } else {
                        CssSelectorPart::PseudoClass(name)
                    }
                }
            }
            NodeType::PseudoElementSelector { value, .. } => CssSelectorPart::PseudoElement(value),
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
                        if let NodeType::Operator(op) = &matcher.node_type {
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
                return Err(CssError::new(format!("Unsupported selector part: {other:?}").as_str()));
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

fn collect_rule(
    prelude: Option<Box<CssNode>>,
    block: Option<Box<CssNode>>,
    media: &[Arc<MediaQueryList>],
) -> CssResult<Option<CssRule>> {
    let mut rule = CssRule {
        selectors: vec![],
        declarations: vec![],
        media: (!media.is_empty()).then(|| media.to_vec()),
    };

    if let Some(node) = prelude {
        let NodeType::SelectorList { selectors } = node.node_type else {
            return Ok(None);
        };

        let mut selector = CssSelector { parts: vec![vec![]] };
        for node in selectors {
            let NodeType::Selector { children } = node.node_type else {
                continue;
            };

            convert_selector_children(children, &mut selector.parts)?;
        }

        // A compound with no parts matches every element vacuously, so an empty prelude
        // (e.g. `/*.a, .b*/{ ... }`, where the whole selector list is commented out) would
        // apply its declarations to the entire document. Per CSS Syntax a style rule with an
        // invalid or empty prelude is invalid and must be dropped, so drop the empty compounds
        // and the rule with them if nothing is left.
        selector.parts.retain(|part| !part.is_empty());
        if selector.parts.is_empty() {
            return Ok(None);
        }

        rule.selectors.push(selector);
    }

    if let Some(declaration) = block {
        let NodeType::Block { children } = declaration.node_type else {
            return Ok(None);
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

            // Convert the nodes into CSS Values
            let mut css_values = vec![];
            for node in value {
                if let Ok(value) = CssValue::parse_ast_node(node) {
                    css_values.push(value);
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

            rule.declarations.push(CssDeclaration {
                property,
                value,
                important,
            });
        }
    }

    Ok(Some(rule))
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
) -> CssResult<()> {
    for node in nodes {
        match node.node_type {
            NodeType::Rule { prelude, block } => {
                if let Some(rule) = collect_rule(prelude, block, media)? {
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
                    let result = collect_rules(children, rules, font_faces, imports, media);
                    media.pop();
                    result?;
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
                        collect_rules(children, rules, font_faces, imports, media)?;
                    }
                }
            }
            NodeType::AtRule {
                name,
                block: Some(block),
                ..
            } if name.eq_ignore_ascii_case("layer") => {
                if let NodeType::Block { children } = block.node_type {
                    collect_rules(children, rules, font_faces, imports, media)?;
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
    Ok(())
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

/// Converts a CSS AST to a CSS stylesheet structure
pub fn convert_ast_to_stylesheet(css_ast: CssNode, origin: CssOrigin, url: &str) -> CssResult<CssStylesheet> {
    let NodeType::StyleSheet { children } = css_ast.node_type else {
        return Err(CssError::new("CSS AST must start with a stylesheet node"));
    };

    let mut sheet = CssStylesheet::new(origin, url);

    collect_rules(
        children,
        &mut sheet.rules,
        &mut sheet.font_faces,
        &mut sheet.imports,
        &mut Vec::new(),
    )?;
    // Recorded once here rather than asked per resize: a sheet using `vw`/`vh` must be
    // restyled whenever the viewport changes, while one that does not can keep its cached
    // computed values (see `CssStylesheet::uses_viewport_units`).
    sheet.uses_viewport_units = sheet
        .rules
        .iter()
        .flat_map(|rule| rule.declarations.iter())
        .any(|decl| decl.value.uses_viewport_units());
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_query::MediaEnvironment;
    use crate::stylesheet::Specificity;
    use crate::Css3;
    use gosub_shared::config::ParserConfig;

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
        assert_eq!(stylesheet.rules[0].declarations.first().unwrap().property, "color");
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
            stylesheet.rules[0].selectors.first().unwrap().parts.len(),
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

        let parts: Vec<_> = stylesheet.rules[0].selectors[0].parts[0].clone();
        assert!(
            parts
                .iter()
                .any(|p| matches!(p, CssSelectorPart::PseudoElement(n) if n == "after")),
            "`:after` must become a pseudo-element, got {parts:?}"
        );

        let parts: Vec<_> = stylesheet.rules[1].selectors[0].parts[0].clone();
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

        let parts = &stylesheet.rules[0].selectors[0].parts[0];
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

        let parts = &stylesheet.rules[0].selectors[0].parts[0];
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

        let spec = |i: usize| Specificity::from(stylesheet.rules[i].selectors[0].parts[0].as_slice());
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
            stylesheet.rules[0].selectors[0].parts[0][0],
            CssSelectorPart::Type("h1".into())
        );
        assert_eq!(
            stylesheet.rules[1].selectors[0].parts[0][0],
            CssSelectorPart::Type("h2".into())
        );
        assert_eq!(
            stylesheet.rules[2].selectors[0].parts[0][0],
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
            sheet.rules[0].selectors[0].parts[0][0],
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
            stylesheet.rules.first().unwrap().declarations.first().unwrap().property,
            "color"
        );
        assert_eq!(
            stylesheet.rules.first().unwrap().declarations.first().unwrap().value,
            CssValue::String("red".into())
        );

        assert_eq!(
            stylesheet.rules.get(1).unwrap().declarations.first().unwrap().property,
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
