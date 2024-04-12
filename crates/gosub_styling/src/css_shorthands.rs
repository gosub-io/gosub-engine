use crate::css_properties::get_property_values;
use gosub_css3::stylesheet::{CssDeclaration, CssValue};

/// This file contains the shorthand properties and their expanded properties.
use lazy_static::lazy_static;
use std::collections::HashMap;

lazy_static! {
    pub static ref SHORTHAND_PROPERTIES: HashMap<&'static str, Vec<&'static str>> = {
        let mut m = HashMap::new();
        m.insert(
            "background",
            vec![
                "background-color",
                "background-image",
                "background-repeat",
                "background-attachment",
                "background-position",
                "background-size",
            ],
        );
        m.insert(
            "border",
            vec!["border-width", "border-style", "border-color"],
        );
        m.insert(
            "border-radius",
            vec![
                "border-top-left-radius",
                "border-top-right-radius",
                "border-bottom-right-radius",
                "border-bottom-left-radius",
            ],
        );
        m.insert(
            "border-width",
            vec![
                "border-top-width",
                "border-right-width",
                "border-bottom-width",
                "border-left-width",
            ],
        );
        m.insert(
            "border-style",
            vec![
                "border-top-style",
                "border-right-style",
                "border-bottom-style",
                "border-left-style",
            ],
        );
        m.insert(
            "border-color",
            vec![
                "border-top-color",
                "border-right-color",
                "border-bottom-color",
                "border-left-color",
            ],
        );
        m.insert(
            "margin",
            vec!["margin-top", "margin-right", "margin-bottom", "margin-left"],
        );
        m.insert(
            "padding",
            vec![
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ],
        );
        m.insert(
            "font",
            vec![
                "font-style",
                "font-variant",
                "font-weight",
                "font-size",
                "line-height",
                "font-family",
            ],
        );
        m.insert(
            "list-style",
            vec!["list-style-type", "list-style-position", "list-style-image"],
        );
        m.insert(
            "outline",
            vec!["outline-width", "outline-style", "outline-color"],
        );
        m.insert("overflow", vec!["overflow-x", "overflow-y"]);
        m.insert(
            "text-decoration",
            vec![
                "text-decoration-line",
                "text-decoration-style",
                "text-decoration-color",
                "text-decoration-thickness",
                "text-underline-position",
            ],
        );
        m.insert(
            "transition",
            vec![
                "transition-property",
                "transition-duration",
                "transition-timing-function",
                "transition-delay",
            ],
        );
        m.insert(
            "border-block-end",
            vec![
                "border-block-end-width",
                "border-block-end-style",
                "border-block-end-color",
            ],
        );
        m.insert(
            "border-block-start",
            vec![
                "border-block-start-width",
                "border-block-start-style",
                "border-block-start-color",
            ],
        );
        m.insert(
            "border-bottom",
            vec![
                "border-bottom-width",
                "border-bottom-style",
                "border-bottom-color",
            ],
        );
        m.insert(
            "border-image",
            vec![
                "border-image-source",
                "border-image-slice",
                "border-image-width",
                "border-image-outset",
                "border-image-repeat",
            ],
        );
        m.insert(
            "border-inline-end",
            vec![
                "border-inline-end-width",
                "border-inline-end-style",
                "border-inline-end-color",
            ],
        );
        m.insert(
            "border-inline-start",
            vec![
                "border-inline-start-width",
                "border-inline-start-style",
                "border-inline-start-color",
            ],
        );
        m.insert(
            "border-left",
            vec![
                "border-left-width",
                "border-left-style",
                "border-left-color",
            ],
        );
        m.insert(
            "border-right",
            vec![
                "border-right-width",
                "border-right-style",
                "border-right-color",
            ],
        );
        m.insert(
            "border-top",
            vec!["border-top-width", "border-top-style", "border-top-color"],
        );
        m.insert(
            "column-rule",
            vec![
                "column-rule-width",
                "column-rule-style",
                "column-rule-color",
            ],
        );
        m.insert("columns", vec!["column-width", "column-count"]);
        m.insert("container", vec!["container-type", "container-name"]);
        m.insert("contain-intrinsic-size", vec!["width", "height"]);
        m.insert("flex", vec!["flex-grow", "flex-shrink", "flex-basis"]);
        m.insert("flex-flow", vec!["flex-direction", "flex-wrap"]);
        m.insert(
            "font-synthesis",
            vec![
                "font-synthesis-weight",
                "font-synthesis-style",
                "font-synthesis-small-caps",
            ],
        );
        m.insert(
            "font-variant",
            vec![
                "font-variant-ligatures",
                "font-variant-caps",
                "font-variant-numeric",
                "font-variant-east-asian",
            ],
        );
        m.insert("gap", vec!["row-gap", "column-gap"]);
        m.insert(
            "grid",
            vec![
                "grid-template-rows",
                "grid-template-columns",
                "grid-template-areas",
                "grid-auto-rows",
                "grid-auto-columns",
                "grid-auto-flow",
            ],
        );
        m.insert(
            "grid-area",
            vec![
                "grid-row-start",
                "grid-column-start",
                "grid-row-end",
                "grid-column-end",
            ],
        );
        m.insert("grid-column", vec!["grid-column-start", "grid-column-end"]);
        m.insert("grid-row", vec!["grid-row-start", "grid-row-end"]);
        m.insert(
            "grid-template",
            vec![
                "grid-template-columns",
                "grid-template-rows",
                "grid-template-areas",
            ],
        );
        m.insert("inset", vec!["top", "right", "bottom", "left"]);
        m.insert(
            "mask",
            vec![
                "mask-image",
                "mask-mode",
                "mask-position",
                "mask-size",
                "mask-repeat",
                "mask-origin",
                "mask-clip",
            ],
        );
        m.insert(
            "mask-border",
            vec![
                "mask-border-source",
                "mask-border-slice",
                "mask-border-width",
                "mask-border-outset",
                "mask-border-repeat",
                "mask-border-mode",
            ],
        );
        m.insert(
            "offset",
            vec![
                "offset-position",
                "offset-path",
                "offset-distance",
                "offset-rotate",
                "offset-anchor",
            ],
        );
        m.insert("place-content", vec!["align-content", "justify-content"]);
        m.insert("place-items", vec!["align-items", "justify-items"]);
        m.insert("place-self", vec!["align-self", "justify-self"]);
        m.insert(
            "scroll-margin",
            vec![
                "scroll-margin-top",
                "scroll-margin-right",
                "scroll-margin-bottom",
                "scroll-margin-left",
            ],
        );
        m.insert(
            "scroll-padding",
            vec![
                "scroll-padding-top",
                "scroll-padding-right",
                "scroll-padding-bottom",
                "scroll-padding-left",
            ],
        );
        m.insert(
            "scroll-timeline",
            vec![
                "timeline-name",
                "timeline-axis",
                "timeline-range",
                "timeline-progress",
            ],
        );
        m.insert(
            "text-emphasis",
            vec!["text-emphasis-style", "text-emphasis-color"],
        );
        m
    };
}

#[allow(dead_code)]
/// Converts a shorthand property to its expanded properties by making sure that the values are set to the correct properties.
fn convert_shorthand_properties(declaration: &CssDeclaration) -> Vec<CssDeclaration> {
    match declaration.property.as_str() {
        "margin" => {
            let values = match declaration.values.len() {
                1 => vec![
                    declaration.values[0].clone(),
                    declaration.values[0].clone(),
                    declaration.values[0].clone(),
                    declaration.values[0].clone(),
                ],
                2 => vec![
                    declaration.values[0].clone(),
                    declaration.values[1].clone(),
                    declaration.values[0].clone(),
                    declaration.values[1].clone(),
                ],
                3 => vec![
                    declaration.values[0].clone(),
                    declaration.values[1].clone(),
                    declaration.values[2].clone(),
                    declaration.values[1].clone(),
                ],
                4 => vec![
                    declaration.values[0].clone(),
                    declaration.values[1].clone(),
                    declaration.values[2].clone(),
                    declaration.values[3].clone(),
                ],
                _ => panic!("Invalid number of values for margin property"),
            };

            let mut declarations = Vec::new();
            declarations.push(CssDeclaration {
                property: "margin-top".to_string(),
                values: vec![values[0].clone()],
                important: declaration.important,
            });
            declarations.push(CssDeclaration {
                property: "margin-left".to_string(),
                values: vec![values[1].clone()],
                important: declaration.important,
            });
            declarations.push(CssDeclaration {
                property: "margin-bottom".to_string(),
                values: vec![values[2].clone()],
                important: declaration.important,
            });
            declarations.push(CssDeclaration {
                property: "margin-right".to_string(),
                values: vec![values[3].clone()],
                important: declaration.important,
            });

            declarations
        }
        "border" => {
            let mut declarations = HashMap::new();
            declarations.insert("border-style", get_initial_values("border-style").unwrap());
            declarations.insert("border-width", get_initial_values("border-width").unwrap());
            declarations.insert("border-color", get_initial_values("border-color").unwrap());

            for value in &declaration.values {
                match value {
                    CssValue::String(_) => {
                        declarations.insert(
                            "border-style",
                            CssDeclaration {
                                property: "border-style".to_string(),
                                values: vec![value.clone()],
                                important: declaration.important,
                            },
                        );
                    }
                    CssValue::Unit(_, _) => {
                        declarations.insert(
                            "border-width",
                            CssDeclaration {
                                property: "border-width".to_string(),
                                values: vec![value.clone()],
                                important: declaration.important,
                            },
                        );
                    }
                    CssValue::Color(_) => {
                        declarations.insert(
                            "border-color",
                            CssDeclaration {
                                property: "border-color".to_string(),
                                values: vec![value.clone()],
                                important: declaration.important,
                            },
                        );
                    }
                    _ => {
                        panic!("Invalid value for border property");
                    }
                }
            }

            declarations.into_values().collect()
        }
        _ => {
            vec![declaration.clone()]
        }
    }
}

#[allow(dead_code)]
/// Returns the default property of the given css property name. Will return none when the property is not found.
fn get_initial_values(name: &str) -> Option<CssDeclaration> {
    if let Some(prop_entry) = get_property_values(name) {
        return Some(CssDeclaration {
            property: name.to_string(),
            values: vec![prop_entry.initial.clone()],
            important: false, // @Todo: is this ok or should it use the declaration's important value?
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_css3::colors::RgbColor;
    use itertools::Itertools;

    macro_rules! border_prop_test {
        ($values:expr, $width:expr, $style:expr, $color:expr,) => {
            println!("Testing border property with values: {:?}", $values);
            let declaration = CssDeclaration {
                property: "border".to_string(),
                values: $values,
                important: false,
            };

            let expanded = convert_shorthand_properties(&declaration);
            assert_eq!(expanded.len(), 3);

            for decl in &expanded {
                match decl.property.as_str() {
                    "border-width" => assert_eq!(decl.values[0], $width),
                    "border-style" => assert_eq!(decl.values[0], $style),
                    "border-color" => assert_eq!(decl.values[0], $color),
                    _ => panic!("Invalid property"),
                }
                assert_eq!(decl.important, false);
            }
        };
    }

    macro_rules! margin_prop_test {
        ($values:expr, $top:expr, $left:expr, $bottom:expr, $right:expr,) => {
            println!("Testing margin property with values: {:?}", $values);
            let declaration = CssDeclaration {
                property: "margin".to_string(),
                values: $values,
                important: false,
            };

            let expanded = convert_shorthand_properties(&declaration);
            assert_eq!(expanded.len(), 4);

            for decl in &expanded {
                match decl.property.as_str() {
                    "margin-top" => assert_eq!(decl.values[0], $top),
                    "margin-left" => assert_eq!(decl.values[0], $left),
                    "margin-bottom" => assert_eq!(decl.values[0], $bottom),
                    "margin-right" => assert_eq!(decl.values[0], $right),
                    _ => panic!("Invalid property"),
                }
                assert_eq!(decl.important, false);
            }
        };
    }

    #[test]
    fn test_convert_shorthand_properties() {
        let list = vec![
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::String("solid".to_string()),
            CssValue::Color(RgbColor::from("#123456")),
        ];

        // lets all permutations of the list
        let permutations = list.iter().permutations(3).collect::<Vec<_>>();
        for permutation in permutations {
            let mut perm_list = Vec::new();
            for perm in permutation {
                perm_list.push(perm.clone());
            }
            border_prop_test!(perm_list, list[0].clone(), list[1].clone(), list[2].clone(),);
        }

        // tests with missing values
        border_prop_test!(
            vec![CssValue::String("solid".to_string()),],
            CssValue::String("initial".to_string()),
            CssValue::String("solid".to_string()),
            CssValue::String("initial".to_string()),
        );

        border_prop_test!(
            vec![CssValue::Unit(1.0, "px".to_string()),],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::String("none".to_string()),
            CssValue::String("initial".to_string()),
        );

        border_prop_test!(
            vec![CssValue::Color(RgbColor::from("#123456")),],
            CssValue::String("initial".to_string()),
            CssValue::String("none".to_string()),
            CssValue::Color(RgbColor::from("#123456")),
        );

        border_prop_test!(
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::String("solid".to_string()),
            ],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::String("solid".to_string()),
            CssValue::String("initial".to_string()),
        );

        border_prop_test!(
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Color(RgbColor::from("#123456")),
            ],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::String("none".to_string()),
            CssValue::Color(RgbColor::from("#123456")),
        );

        border_prop_test!(
            vec![
                CssValue::String("solid".to_string()),
                CssValue::Color(RgbColor::from("#123456")),
            ],
            CssValue::String("initial".to_string()),
            CssValue::String("solid".to_string()),
            CssValue::Color(RgbColor::from("#123456")),
        );
    }

    #[test]
    fn test_margin() {
        margin_prop_test!(
            vec![CssValue::Unit(1.0, "px".to_string()),],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(1.0, "px".to_string()),
        );

        margin_prop_test!(
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
            ],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
        );

        margin_prop_test!(
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
                CssValue::Unit(3.0, "px".to_string()),
            ],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
            CssValue::Unit(3.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
        );

        margin_prop_test!(
            vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
                CssValue::Unit(3.0, "px".to_string()),
                CssValue::Unit(4.0, "px".to_string()),
            ],
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Unit(2.0, "px".to_string()),
            CssValue::Unit(3.0, "px".to_string()),
            CssValue::Unit(4.0, "px".to_string()),
        );
    }

    #[test]
    fn test_get_initial_values() {
        let initial = get_initial_values("border-style");

        assert!(initial.is_some());
        assert_eq!(
            initial.unwrap().values[0],
            CssValue::String("none".to_string())
        );
    }
}
