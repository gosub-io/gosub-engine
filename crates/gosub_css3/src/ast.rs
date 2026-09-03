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
                    NodeType::PseudoClassSelector { value, .. } => CssSelectorPart::PseudoClass(value.to_string()),
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
                        selector.parts.push(vec![]);
                        continue;
                    }
                    other => {
                        return Err(CssError::new(format!("Unsupported selector part: {other:?}").as_str()));
                    }
                };
                if let Some(x) = selector.parts.last_mut() {
                    x.push(part);
                } else {
                    selector.parts.push(vec![part]); //unreachable, but still, we handle it
                }
            }
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
                for n in value_nodes {
                    if let Ok(v) = CssValue::parse_ast_node(n) {
                        collect_src_urls(&v, &mut sources);
                    }
                }
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

/// Recursively collect `url(...)` targets from an `@font-face` `src` value.
fn collect_src_urls(value: &CssValue, out: &mut Vec<String>) {
    match value {
        CssValue::Function(name, args) if name.eq_ignore_ascii_case("url") => {
            if let Some(url) = args.iter().find_map(|a| match a {
                CssValue::String(s) => Some(s.trim_matches(['"', '\'']).to_string()),
                _ => None,
            }) {
                if !url.is_empty() {
                    out.push(url);
                }
            }
        }
        CssValue::List(list) => {
            for item in list {
                collect_src_urls(item, out);
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
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_query::MediaEnvironment;
    use crate::Css3;
    use gosub_shared::config::ParserConfig;

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
