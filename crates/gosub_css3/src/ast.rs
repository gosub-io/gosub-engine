use log::warn;

use crate::node::{Node as CssNode, NodeType};
use crate::stylesheet::{
    AttributeSelector, Combinator, CssDeclaration, CssRule, CssSelector, CssSelectorPart, CssStylesheet, CssValue,
    FontFace, MatcherType,
};
use gosub_interface::css3::CssOrigin;
use gosub_shared::errors::{CssError, CssResult};

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

fn collect_rule(node: &CssNode) -> CssResult<Option<CssRule>> {
    let mut rule = CssRule {
        selectors: vec![],
        declarations: vec![],
    };

    let (prelude, declarations) = node.as_rule();
    if let Some(node) = prelude {
        if !node.is_selector_list() {
            return Ok(None);
        }

        let mut selector = CssSelector { parts: vec![vec![]] };
        for node in node.as_selector_list() {
            if !node.is_selector() {
                continue;
            }

            for node in node.as_selector() {
                let part = match &*node.node_type {
                    NodeType::Ident { value } => CssSelectorPart::Type(value.clone()),
                    NodeType::ClassSelector { value } => CssSelectorPart::Class(value.clone()),
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
                    NodeType::IdSelector { value } => CssSelectorPart::Id(value.clone()),
                    NodeType::TypeSelector { value, .. } if value == "*" => CssSelectorPart::Universal,
                    NodeType::PseudoClassSelector { value, .. } => CssSelectorPart::PseudoClass(value.to_string()),
                    NodeType::PseudoElementSelector { value, .. } => CssSelectorPart::PseudoElement(value.to_string()),
                    NodeType::TypeSelector { value, .. } => CssSelectorPart::Type(value.clone()),
                    NodeType::AttributeSelector {
                        name,
                        value,
                        flags,
                        matcher,
                    } => {
                        let matcher = match matcher {
                            None => MatcherType::None,

                            Some(matcher) => {
                                if let NodeType::Operator(op) = &*matcher.node_type {
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
                            name: name.clone(),
                            matcher,
                            value: value.clone(),
                            case_insensitive: flags.eq_ignore_ascii_case("i"),
                        }))
                    }
                    NodeType::Comma => {
                        selector.parts.push(vec![]);
                        continue;
                    }
                    _ => {
                        return Err(CssError::new(
                            format!("Unsupported selector part: {:?}", node.node_type).as_str(),
                        ));
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

    if let Some(declaration) = declarations {
        if !declaration.is_block() {
            return Ok(None);
        }

        let block = declaration.as_block();
        for declaration in block {
            if !declaration.is_declaration() {
                continue;
            }

            let (property, nodes, important) = declaration.as_declaration();

            // Convert the nodes into CSS Values
            let mut css_values = vec![];
            for node in nodes {
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
                property: property.clone(),
                value,
                important: *important,
            });
        }
    }

    Ok(Some(rule))
}

fn collect_rules(nodes: &[CssNode], rules: &mut Vec<CssRule>, font_faces: &mut Vec<FontFace>) -> CssResult<()> {
    for node in nodes {
        match &*node.node_type {
            NodeType::Rule { .. } => {
                if let Some(rule) = collect_rule(node)? {
                    rules.push(rule);
                }
            }
            NodeType::AtRule {
                name,
                block: Some(block),
                ..
            } if name.eq_ignore_ascii_case("layer") => {
                collect_rules(block.as_block(), rules, font_faces)?;
            }
            NodeType::AtRule {
                name,
                block: Some(block),
                ..
            } if name.eq_ignore_ascii_case("font-face") => {
                if let Some(face) = collect_font_face(block.as_block()) {
                    font_faces.push(face);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Build a [`FontFace`] from the declarations inside an `@font-face` block. Requires a
/// `font-family` and at least one `src: url(...)`; returns `None` otherwise.
fn collect_font_face(nodes: &[CssNode]) -> Option<FontFace> {
    let mut family: Option<String> = None;
    let mut sources: Vec<String> = Vec::new();
    let mut unicode_range: Option<String> = None;

    for decl in nodes {
        if !decl.is_declaration() {
            continue;
        }
        let (property, value_nodes, _important) = decl.as_declaration();
        match property.to_ascii_lowercase().as_str() {
            "font-family" => {
                let name: String = value_nodes
                    .iter()
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
                    .iter()
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
pub fn convert_ast_to_stylesheet(css_ast: &CssNode, origin: CssOrigin, url: &str) -> CssResult<CssStylesheet> {
    if !css_ast.is_stylesheet() {
        return Err(CssError::new("CSS AST must start with a stylesheet node"));
    }

    let mut sheet = CssStylesheet {
        rules: vec![],
        font_faces: vec![],
        origin,
        url: url.to_string(),
        parse_log: vec![],
    };

    collect_rules(css_ast.as_stylesheet(), &mut sheet.rules, &mut sheet.font_faces)?;
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
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
