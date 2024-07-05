use crate::node::{Node as CssNode, NodeType};
use crate::stylesheet::{
    CssDeclaration, CssOrigin, CssRule, CssSelector, CssSelectorPart, CssSelectorType,
    CssStylesheet, CssValue, MatcherType,
};
use anyhow::anyhow;
use gosub_shared::types::Result;

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

            let mut selector = CssSelector { parts: vec![] };
            for node in node.as_selector_list().iter() {
                if !node.is_selector() {
                    continue;
                }

                for node in node.as_selector() {
                    let part = match &*node.node_type {
                        NodeType::Ident { value } => CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::ClassSelector { value } => CssSelectorPart {
                            type_: CssSelectorType::Class,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::Combinator { value } => CssSelectorPart {
                            type_: CssSelectorType::Combinator,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::IdSelector { value } => CssSelectorPart {
                            type_: CssSelectorType::Id,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::TypeSelector { value, .. } if value == "*" => CssSelectorPart {
                            type_: CssSelectorType::Universal,
                            value: "*".to_string(),
                            ..Default::default()
                        },
                        NodeType::PseudoClassSelector { value, .. } => CssSelectorPart {
                            type_: CssSelectorType::PseudoClass,
                            value: value.to_string(),
                            ..Default::default()
                        },
                        NodeType::PseudoElementSelector { value, .. } => CssSelectorPart {
                            type_: CssSelectorType::PseudoElement,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::TypeSelector { value, .. } => CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: value.clone(),
                            ..Default::default()
                        },
                        NodeType::AttributeSelector {
                            name, value, flags, ..
                        } => CssSelectorPart {
                            type_: CssSelectorType::Attribute,
                            name: name.clone(),
                            matcher: MatcherType::Equals, // @todo: this needs to be parsed
                            value: value.clone(),
                            flags: flags.clone(),
                        },
                        _ => {
                            panic!("Unknown selector type: {:?}", node);
                        }
                    };
                    selector.parts.push(part);
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

                if css_values.len() > 1 {
                    rule.declarations.push(CssDeclaration {
                        property: property.clone(),
                        value: CssValue::List(css_values.to_vec()),
                        important: *important,
                    });
                } else {
                    rule.declarations.push(CssDeclaration {
                        property: property.clone(),
                        value: css_values.first().expect("").clone(),
                        important: *important,
                    });
                }
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
            CssValue::String("red".into())
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
            CssValue::List(vec![
                CssValue::Unit(1.0, "px".into()),
                CssValue::String("solid".into()),
                CssValue::String("black".into())
            ])
        );
    }
}
