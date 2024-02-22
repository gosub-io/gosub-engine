use crate::node::Node as CssNode;
use crate::stylesheet::{
    CssDeclaration, CssOrigin, CssRule, CssSelector, CssSelectorPart, CssSelectorType,
    CssStylesheet, MatcherType,
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
                    if node.is_ident() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: node.as_ident().clone(),
                            ..Default::default()
                        });
                    } else if node.is_class_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Class,
                            value: node.as_class_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_combinator() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Combinator,
                            value: node.as_combinator().clone(),
                            ..Default::default()
                        });
                    } else if node.is_id_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Id,
                            value: node.as_id_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_universal_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Universal,
                            value: "*".to_string(),
                            ..Default::default()
                        });
                    } else if node.is_pseudo_class_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::PseudoClass,
                            value: node.as_pseudo_class_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_pseudo_element_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::PseudoElement,
                            value: node.as_pseudo_element_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_type_selector() {
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Type,
                            value: node.as_type_selector().clone(),
                            ..Default::default()
                        });
                    } else if node.is_attribute_selector() {
                        let attr_selector = node.as_attribute_selector();
                        selector.parts.push(CssSelectorPart {
                            type_: CssSelectorType::Attribute,
                            name: attr_selector.0.clone(),
                            matcher: MatcherType::Equals, // @todo: this needs to be parsed
                            value: attr_selector.2.clone(),
                            flags: attr_selector.3.clone(),
                        });
                    } else {
                        panic!("Unknown selector type: {:?}", node);
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

                let (property, value, important) = declaration.as_declaration();
                rule.declarations.push(CssDeclaration {
                    property: property.clone(),
                    value: value[0].to_string(),
                    important: *important,
                });
            }
        }

        sheet.rules.push(rule);
    }
    Ok(sheet)
}
