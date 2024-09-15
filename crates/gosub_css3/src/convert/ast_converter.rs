use crate::node::{Node as CssNode, NodeType};
use crate::stylesheet::{
    AttributeSelector, Combinator, CssDeclaration, CssOrigin, CssRule, CssSelector,
    CssSelectorPart, CssStylesheet, CssValue, MatcherType,
};
use anyhow::anyhow;
use gosub_shared::types::Result;
use log::warn;

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

/// Converts a CSS AST to a CSS stylesheet structure
pub fn convert_ast_to_stylesheet(
    css_ast: &CssNode,
    origin: CssOrigin,
    location: &str,
) -> Result<CssStylesheet> {
    if !css_ast.is_stylesheet() {
        return Err(anyhow!("CSS AST must start with a stylesheet node"));
    }

    let mut sheet = CssStylesheet {
        rules: vec![],
        origin,
        location: location.to_string(),
    };

    for node in css_ast.as_stylesheet() {
        if !node.is_rule() {
            continue;
        }

        let mut rule = CssRule {
            selectors: vec![],
            declarations: vec![],
        };

        let (prelude, declarations) = node.as_rule();
        for node in prelude.iter() {
            if !node.is_selector_list() {
                continue;
            }

            let mut selector = CssSelector {
                parts: vec![vec![]],
            };
            for node in node.as_selector_list().iter() {
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
                                _ => return Err(anyhow!("Unknown combinator: {}", value)),
                            };

                            CssSelectorPart::Combinator(combinator)
                        }
                        NodeType::IdSelector { value } => CssSelectorPart::Id(value.clone()),
                        NodeType::TypeSelector { value, .. } if value == "*" => {
                            CssSelectorPart::Universal
                        }
                        NodeType::PseudoClassSelector { value, .. } => {
                            CssSelectorPart::PseudoClass(value.to_string())
                        }
                        NodeType::PseudoElementSelector { value, .. } => {
                            CssSelectorPart::PseudoElement(value.to_string())
                        }
                        NodeType::TypeSelector { value, .. } => {
                            CssSelectorPart::Type(value.clone())
                        }
                        NodeType::AttributeSelector {
                            name,
                            value,
                            flags,
                            matcher,
                        } => {
                            let matcher = match matcher {
                                None => MatcherType::None,

                                Some(matcher) => match &*matcher.node_type {
                                    NodeType::Operator(op) => match op.as_str() {
                                        "=" => MatcherType::Equals,
                                        "~=" => MatcherType::Includes,
                                        "|=" => MatcherType::DashMatch,
                                        "^=" => MatcherType::PrefixMatch,
                                        "$=" => MatcherType::SuffixMatch,
                                        "*=" => MatcherType::SubstringMatch,
                                        _ => {
                                            warn!("Unsupported matcher: {:?}", matcher);
                                            MatcherType::Equals
                                        }
                                    },
                                    _ => {
                                        warn!("Unsupported matcher: {:?}", matcher);
                                        MatcherType::Equals
                                    }
                                },
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
                            return Err(anyhow!("Unsupported selector part: {:?}", node.node_type))
                        }
                    };
                    if let Some(x) = selector.parts.last_mut() {
                        x.push(part)
                    } else {
                        selector.parts.push(vec![part]); //unreachable, but still, we handle it
                    }
                }
            }
            rule.selectors.push(selector);
        }

        for declaration in declarations.iter() {
            if !declaration.is_block() {
                continue;
            }

            let block = declaration.as_block();
            for declaration in block.iter() {
                if !declaration.is_declaration() {
                    continue;
                }

                let (property, nodes, important) = declaration.as_declaration();

                // Convert the nodes into CSS Values
                let mut css_values = vec![];
                for node in nodes.iter() {
                    if let Ok(value) = CssValue::parse_ast_node(node) {
                        css_values.push(value);
                    }
                }

                if css_values.is_empty() {
                    continue;
                }

                rule.declarations.push(CssDeclaration {
                    property: property.clone(),
                    value: css_values.to_vec(),
                    important: *important,
                });
            }
        }

        sheet.rules.push(rule);
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser_config::ParserConfig;
    use crate::Css3;

    #[test]
    fn convert_font_family() {
        let ast = Css3::parse(
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
        )
        .unwrap();

        let tree = convert_ast_to_stylesheet(&ast, CssOrigin::UserAgent, "test.css").unwrap();

        dbg!(&tree);
    }

    #[test]
    fn convert_test() {
        let ast = Css3::parse(
            r#"
            h1 { color: red; }
            h3, h4 { border: 1px solid black; }
            "#,
            ParserConfig::default(),
        )
        .unwrap();

        let tree = convert_ast_to_stylesheet(&ast, CssOrigin::UserAgent, "test.css").unwrap();

        assert_eq!(
            tree.rules
                .first()
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .property,
            "color"
        );
        assert_eq!(
            tree.rules
                .first()
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .value,
            vec![CssValue::String("red".into())]
        );

        assert_eq!(
            tree.rules
                .get(1)
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .property,
            "border"
        );
        assert_eq!(
            tree.rules
                .get(1)
                .unwrap()
                .declarations
                .first()
                .unwrap()
                .value,
            vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::String("black".into())
            ]
        );
    }
}
