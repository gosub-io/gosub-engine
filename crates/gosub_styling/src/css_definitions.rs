use gosub_css3::stylesheet::CssValue;
use memoize::memoize;
use std::collections::HashMap;
use log::warn;
use crate::syntax::CssSyntax;
use crate::syntax_matcher::CssSyntaxTree;

/// A CSS property definition including its type and initial value and optional expanded values if it's a shorthand property
#[derive(Debug, Clone)]
pub struct PropertyDefinition {
    /// Name of the property (ie: color, background etc)
    name: String,
    /// List of expanded properties if this property is a shorthand property
    expanded_properties: Vec<String>,
    /// Syntax tree of the property. This is a tree that describes the valid values for this property.
    syntax: CssSyntaxTree,
    /// True when the property inherits from parent nodes if not set
    inherits: bool,
    /// Initial value of the property
    initial_value: CssValue,
}

impl PropertyDefinition {
    pub fn name(self) -> String {
        self.name.clone()
    }

    pub fn expanded_properties(self) -> Vec<String> {
        self.expanded_properties.clone()
    }

    pub fn syntax(self) -> CssSyntaxTree {
        self.syntax
    }

    pub fn inherits(self) -> bool {
        self.inherits
    }

    pub fn initial_value(self) -> CssValue {
        self.initial_value.clone()
    }

    /// Matches a list of values against the current definition
    pub fn matches(&self, value: &CssValue) -> Option<CssValue> {
        self.syntax.matches(value)
    }

    // /// Matches a string against the current definition
    // pub fn matches_str(&self, value: &str) -> Option<CssValue> {
    //     self.syntax.matches_str(value)
    // }

    // /// Parses and orders the values based on the order of the types. Assumes (must!) have unique types for each value.
    // /// ie. "color, string, unit" is ok, but "color, number, number" isn't.
    // fn parser_any_order_is_ok(&self, values: &Vec<CssValue>) -> Result<Vec<CssValue>> {
    //     println!("Checking any_order_is_ok");
    //     dbg!(&values);
    //     dbg!(&self.syntax);
    //     Ok(values.clone())
    // }
    //
    // /// Parses and orders the values based on their count. This is mostly used for "top, right, bottom, left" properties.
    // /// When one value is found, it's used for all the 4 values.
    // /// When two values are found, the first is used for top and bottom, the second for right and left.
    // /// When three values are found, the first is used for top, the second for right and left, the third for bottom.
    // /// When four values are found, they are used in that order.
    // fn parser_one_to_four(&self, values: &Vec<CssValue>) -> Result<Vec<CssValue>> {
    //     println!("Parser one_to_four");
    //
    //     if values.len() < 1 || values.len() > 4 {
    //         return Err(Error::CssGeneric(format!("Expected 1 to 4 values, got {}", values.len())).into());
    //     }
    //
    //     match self.parser_default(values) {
    //         Ok(values) => {
    //             match values.len() {
    //                 1 => {
    //                     return Ok(vec![
    //                         values[0].clone(),
    //                         values[0].clone(),
    //                         values[0].clone(),
    //                         values[0].clone(),
    //                     ]);
    //                 }
    //                 2 => {
    //                     return Ok(vec![
    //                         values[0].clone(),
    //                         values[1].clone(),
    //                         values[0].clone(),
    //                         values[1].clone(),
    //                     ]);
    //                 }
    //                 3 => {
    //                     return Ok(vec![
    //                         values[0].clone(),
    //                         values[1].clone(),
    //                         values[2].clone(),
    //                         values[1].clone(),
    //                     ]);
    //                 }
    //                 4 => {
    //                     return Ok(values);
    //                 }
    //                 _ => {
    //                     return Err(Error::CssGeneric(format!("Expected 1 to 4 values, got {}", values.len())).into());
    //                 }
    //             }
    //         }
    //         Err(e) => {
    //             return Err(e);
    //         }
    //     }
    //  }

    pub fn check_expanded_properties(&self, _values: &[CssValue]) -> bool {
        // if values.len() != self.expanded_properties.len() {
        //     return false;
        // }
        //
        // for (i, value) in values.iter().enumerate() {
        //     let prop = self.expanded_properties.get(i).unwrap();
        //     let prop_def = parse_definition_file().find(prop).unwrap();
        //     if !prop_def.matches(&vec![value.clone()]) {
        //         return false;
        //     }
        // }

        true
    }

    // /// This is the default parser. It will check if any of the values match any of the types.
    // /// It doesn't need to match everything ("number|color" -> 42 or red is ok, 42px is not)
    // pub fn parser_default(&self, values: &Vec<CssValue>) -> Result<Vec<CssValue>> {
    //     // We check each value. All values must match at least one of the types
    //     for value in values.iter() {
    //         println!("Checking value: {:?}", value);
    //         for syntax in &self.type_ {
    //             println!("Checking against type: {:?}", type_);
    //             if !check_type(type_, Some(value)) {
    //                 return Err(Error::CssGeneric(format!("Value {:?} does not match type {:?}", value, type_)).into());
    //             }
    //         }
    //     }
    //
    //     return Ok(values.clone());
    // }
}

#[derive(Clone)]
pub struct CssPropertyDefinitions {
    definitions: HashMap<String, PropertyDefinition>,
}

impl Default for CssPropertyDefinitions {
    fn default() -> Self {
        Self::new()
    }
}

impl CssPropertyDefinitions {
    pub fn new() -> Self {
        parse_definition_files()
    }

    pub fn empty() -> Self {
        CssPropertyDefinitions {
            definitions: HashMap::new(),
        }
    }

    pub fn add_definition(&mut self, name: &str, definition: PropertyDefinition) {
        self.definitions.insert(name.to_string(), definition);
    }


    pub fn find(&self, name: &str) -> Option<PropertyDefinition> {
        self.definitions.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }

    pub fn get_definitions(self) -> HashMap<String, PropertyDefinition> {
        self.definitions.clone()
    }
}

/// A collection of CSS property type definitions
#[derive(Clone)]
pub struct CssPropertyTypeDefs {
    typedefs: HashMap<String, CssPropertyTypeDef>,
}

impl Default for CssPropertyTypeDefs {
    fn default() -> Self {
        Self::new()
    }
}

impl CssPropertyTypeDefs {
    pub fn new() -> Self {
        CssPropertyTypeDefs {
            typedefs: HashMap::new(),
        }
    }

    pub fn add_typedef(&mut self, name: &str, typedef: CssPropertyTypeDef) {
        self.typedefs.insert(name.to_string(), typedef);
    }

    pub fn find(&self, name: &str) -> Option<CssPropertyTypeDef> {
        self.typedefs.get(name).cloned()
    }

    pub fn len(&self) -> usize {
        self.typedefs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.typedefs.is_empty()
    }

    pub fn get_typedefs(self) -> HashMap<String, CssPropertyTypeDef> {
        self.typedefs.clone()
    }
}

/// A CSS property definition including its type and initial value and optional expanded values if it's a shorthand property
#[derive(Debug, Clone)]
pub struct CssPropertyTypeDef {
    /// Name of the property (ie: color, background etc)
    name: String,
    /// Syntax tree of the property. This is a tree that describes the valid values for this property.
    syntax: CssSyntaxTree,
}

impl CssPropertyTypeDef {
    pub fn name(self) -> String {
        self.name.clone()
    }

    pub fn syntax(self) -> CssSyntaxTree {
        self.syntax
    }
}


/// Parses the internal CSS definition file
#[memoize]
pub fn parse_definition_files() -> CssPropertyDefinitions {
    // First, parse all typedefs so we can use them in the definitions
    let contents = include_str!("../resources/css_typedefs.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let typedefs = parse_typedef_file_internal(json);

    // Parse definitions
    let contents = include_str!("../resources/css_definitions.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    parse_definition_file_internal(json, typedefs)
}

pub fn get_css_definitions() -> CssPropertyDefinitions {
    parse_definition_files()
}

// /// Converts a property value to a CSS value. This is a special case for some properties.
// fn to_value_from_prop(property: &str, value: &str) -> Result<CssValue> {
//     // Some of the properties have special parsing rules
//     match property {
//         "background-position" => to_value_background_position(value),
//         _ => to_value(value),
//     }
// }

// // Parse the special case "background position". It consists of 2 parts, both of which can be a number, percentage or string
// fn to_value_background_position(value: &str) -> Result<CssValue> {
//     let mut parts: Vec<&str> = value.split(' ').collect();
//
//     let v1 = CssValue::parse(parts[0]);
//     if v1.is_err() {
//         return Err(Error::CssGeneric(format!("Could not convert value: {}", value)).into());
//     }
//
//     // Make sure we have always 2 parts. The second part depends on what the type of the first part is.
//     if parts.len() == 1 {
//         match v1 {
//             Ok(CssValue::Unit(_, _)) => parts.push("50%"),
//             Ok(CssValue::Percentage(_)) => parts.push("50%"),
//             Ok(CssValue::String(_)) => parts.push("center"),
//             _ => {
//                 return Err(Error::CssGeneric(format!("Could not convert value: {}", value)).into())
//             }
//         }
//     }
//
//     let v2 = to_value(parts[1]);
//     if v2.is_err() {
//         return Err(Error::CssGeneric(format!("Could not convert value: {}", value)).into());
//     }
//
//     // background position is always returned as a list[2]
//     Ok(CssValue::List(vec![v1.unwrap(), v2.unwrap()]))
// }

/// Parses a typedef JSON import file
fn parse_typedef_file_internal(json: serde_json::Value) -> CssPropertyTypeDefs {
    let mut typedefs = HashMap::new();

    let entries = json.as_object().unwrap();
    for (name, entry) in entries.iter() {
        match CssSyntax::new(entry.as_str().unwrap()).compile() {
            Ok(ast) => {
                typedefs.insert(
                    name.clone(),
                    CssPropertyTypeDef {
                        name: name.clone(),
                        syntax: ast.clone(),
                    },
                );
            }
            Err(e) => {
                warn!("Could not compile syntax for typedef {:?}: {:?}", name, e);
                // panic!("Could not compile syntax for typedef {:?}: {:?}", name, e);
            }
        }
    }

    CssPropertyTypeDefs { typedefs }
}


/// Parses the JSON input into a CSS property definitions structure
fn parse_definition_file_internal(json: serde_json::Value, _typedefs: CssPropertyTypeDefs) -> CssPropertyDefinitions {
    let mut definitions = HashMap::new();

    let entries = json.as_array().unwrap();
    for entry in entries {
        let name = entry["name"].as_str().unwrap().to_string();

        let mut expanded_properties = vec![];
        let mut syntax = CssSyntaxTree::new(vec![]);
        let mut inherits: bool = false;
        let mut initial_value: CssValue = CssValue::None;

        if let Some(value) = entry["expanded_properties"].as_array() {
            expanded_properties = value
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
        }
        if let Some(value) = entry["syntax"].as_str() {
            if let Ok(ast) = CssSyntax::new(value).compile() {
                syntax = ast.clone();
            } else {
                warn!("Could not compile syntax {:?}", entry);
                // panic!("Could not compile syntax {:?}", entry);
            }
        }
        if let Some(value) = entry["inherits"].as_bool() {
            inherits = value;
        }
        if let Ok(value) = CssValue::parse_str(entry["initial_value"].as_str().unwrap_or("none")) {
            match syntax.matches(&value) {
                Some(v) => initial_value = v,
                None => {
                    warn!("Cannot validate initial value {:?} against syntax for property {}", entry, name);
                }
            }
        }

        // Sanity checks
        if !expanded_properties.is_empty() && initial_value != CssValue::None {
            panic!(
                "Expanded properties and initial value are mutually exclusive {:?}",
                entry
            );
        }
        // if expanded_properties.len() > 0 && syntax.len() > 0 {
        //     panic!("Expanded properties and type are mutually exclusive {:?}", entry);
        // }

        definitions.insert(
            name.clone(),
            PropertyDefinition {
                name: name.clone(),
                expanded_properties,
                syntax,
                inherits,
                initial_value,
            },
        );
    }

    CssPropertyDefinitions { definitions }
}

#[cfg(test)]
mod tests {
    use gosub_css3::colors::RgbColor;
    use super::*;

    #[test]
    fn test_parse_definition_file() {
        let definitions = parse_definition_files();
        assert_eq!(definitions.len(), 115);
    }

    // #[test]
    // fn test_to_value_background_position() {
    //     struct Provider {
    //         input: String,
    //         exp1: CssValue,
    //         exp2: CssValue,
    //     }
    //
    //     let provider = vec![
    //         Provider {
    //             input: "center".into(),
    //             exp1: CssValue::String("center".into()),
    //             exp2: CssValue::String("center".into()),
    //         },
    //         Provider {
    //             input: "left".into(),
    //             exp1: CssValue::String("left".into()),
    //             exp2: CssValue::String("center".into()),
    //         },
    //         Provider {
    //             input: "right".into(),
    //             exp1: CssValue::String("right".into()),
    //             exp2: CssValue::String("center".into()),
    //         },
    //         Provider {
    //             input: "center center".into(),
    //             exp1: CssValue::String("center".into()),
    //             exp2: CssValue::String("center".into()),
    //         },
    //         Provider {
    //             input: "left left".into(),
    //             exp1: CssValue::String("left".into()),
    //             exp2: CssValue::String("left".into()),
    //         },
    //         Provider {
    //             input: "left right".into(),
    //             exp1: CssValue::String("left".into()),
    //             exp2: CssValue::String("right".into()),
    //         },
    //         Provider {
    //             input: "left 5%".into(),
    //             exp1: CssValue::String("left".into()),
    //             exp2: CssValue::Percentage(5.0),
    //         },
    //         Provider {
    //             input: "center 5%".into(),
    //             exp1: CssValue::String("center".into()),
    //             exp2: CssValue::Percentage(5.0),
    //         },
    //         Provider {
    //             input: "10% 10%".into(),
    //             exp1: CssValue::Percentage(10.0),
    //             exp2: CssValue::Percentage(10.0),
    //         },
    //         Provider {
    //             input: "10%".into(),
    //             exp1: CssValue::Percentage(10.0),
    //             exp2: CssValue::Percentage(50.0),
    //         },
    //         // This case is not valid
    //         Provider {
    //             input: "10% left".into(),
    //             exp1: CssValue::Percentage(10.0),
    //             exp2: CssValue::String("left".to_string()),
    //         },
    //         Provider {
    //             input: "50px 50px".into(),
    //             exp1: CssValue::Unit(50.0, "px".into()),
    //             exp2: CssValue::Unit(50.0, "px".into()),
    //         },
    //         Provider {
    //             input: "50px 10%".into(),
    //             exp1: CssValue::Unit(50.0, "px".into()),
    //             exp2: CssValue::Percentage(10.0),
    //         },
    //         Provider {
    //             input: "50px".into(),
    //             exp1: CssValue::Unit(50.0, "px".into()),
    //             exp2: CssValue::Percentage(50.0),
    //         },
    //         Provider {
    //             input: "50px 1.4em".into(),
    //             exp1: CssValue::Unit(50.0, "px".into()),
    //             exp2: CssValue::Unit(1.4, "em".into()),
    //         },
    //     ];
    //
    //     for p in provider {
    //         let value = to_value_background_position(p.input.as_str()).unwrap();
    //         assert_eq!(value, CssValue::List(vec![p.exp1.clone(), p.exp2.clone()]));
    //     }
    // }

    // #[test]
    // fn test_to_value() {
    //     assert_eq!(
    //         CssPropertyDefinition::from_str("hello").unwrap(),
    //         CssValue::String("hello".to_string())
    //     );
    //     assert_eq!(
    //         to_value("color(#ff0000)").unwrap(),
    //         CssValue::Color(RgbColor::from("#ff0000"))
    //     );
    //     assert_eq!(
    //         to_value("color(#ff0000)").unwrap(),
    //         CssValue::Color(RgbColor::from("red"))
    //     );
    //     assert_eq!(
    //         to_value("color(rebeccapurple)").unwrap(),
    //         CssValue::Color(RgbColor::from("#663399"))
    //     );
    //     assert_eq!(to_value("42").unwrap(), CssValue::Number(42.0));
    //     assert_eq!(to_value("12.34").unwrap(), CssValue::Number(12.34));
    //     assert_eq!(to_value("64.8%").unwrap(), CssValue::Percentage(64.8));
    //     assert_eq!(
    //         to_value("42px").unwrap(),
    //         CssValue::Unit(42.0, "px".to_string())
    //     );
    //     assert_eq!(to_value("none").unwrap(), CssValue::None);
    // }

    #[test]
    fn test_prop_def() {
        let definitions = parse_definition_files();

        let prop = definitions.find("color").unwrap();
        assert!(prop.matches(&CssValue::Color(RgbColor::from("#ff0000"))).is_some());
        assert!(!prop.matches(&CssValue::Number(42.0)).is_some());

        let prop = definitions.find("border").unwrap();
        assert!(prop.matches(&CssValue::List(vec![
            CssValue::Color(RgbColor::from("black")),
            CssValue::String("solid".into()),
            CssValue::Unit(1.0, "px".into()),
        ])).is_some());
        assert!(prop.matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
            CssValue::Unit(1.0, "px".into()),
        ])).is_some());
        assert!(prop.matches(&CssValue::Unit(1.0, "px".into())).is_some());
        assert!(prop.matches(&CssValue::String("solid".into())).is_some());
        assert!(prop.matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
        ])).is_some());
        assert!(prop.matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
        ])).is_some());
        assert!(prop.matches(&CssValue::String("solid".into())).is_some());
    }

    // #[test]
    // fn test_str_to_css_value() {
    //     assert_eq!(
    //         convert_str_to_css_value("'hello'").unwrap(),
    //         CssValue::String("hello".to_string())
    //     );
    //     assert_eq!(
    //         convert_str_to_css_value("color(#ff0000)").unwrap(),
    //         CssValue::Color(RgbColor::from("#ff0000"))
    //     );
    //     assert_eq!(
    //         convert_str_to_css_value("color(#ff0000)").unwrap(),
    //         CssValue::Color(RgbColor::from("red"))
    //     );
    //     assert_eq!(
    //         convert_str_to_css_value("color(rebeccapurple)").unwrap(),
    //         CssValue::Color(RgbColor::from("#663399"))
    //     );
    //     assert_eq!(convert_str_to_css_value("42").unwrap(), CssValue::Number(42.0));
    //     assert_eq!(convert_str_to_css_value("12.34").unwrap(), CssValue::Number(12.34));
    //     assert_eq!(convert_str_to_css_value("64.8%").unwrap(), CssValue::Percentage(64.8));
    //     assert_eq!(
    //         convert_str_to_css_value("unit(42 px)").unwrap(),
    //         CssValue::Unit(42.0, "px".to_string())
    //     );
    //     assert_eq!(convert_str_to_css_value("none").unwrap(), CssValue::None);
    //
    //     assert_eq!(convert_str_to_css_value("10px").unwrap(), CssValue::Unit(10.0, "px".to_string()));
    //
    //     assert!(convert_str_to_css_value("does-not-exists").is_err());
    // }

    #[test]
    fn test_property_definitions() {
        let mut definitions = CssPropertyDefinitions::empty();
        definitions.add_definition(
            "color",
            PropertyDefinition {
                name: "color".to_string(),
                expanded_properties: vec![],
                syntax: CssSyntax::new("color()".into()).compile().expect("Could not compile syntax"),
                inherits: false,
                initial_value: CssValue::None,
            },
        );

        assert_eq!(definitions.len(), 1);
        assert!(definitions.find("color").is_some());
        assert!(definitions.find("border-top-style").is_none());

        definitions.add_definition(
            "border-style",
            PropertyDefinition {
                name: "border-style".to_string(),
                expanded_properties: vec![
                    "border-top-style".to_string(),
                    "border-right-style".to_string(),
                    "border-bottom-style".to_string(),
                    "border-left-style".to_string(),
                ],
                syntax: CssSyntax::new("".into()).compile().expect("Could not compile syntax"),
                inherits: false,
                initial_value: CssValue::String("thick".to_string()),
            },
        );

        assert_eq!(definitions.len(), 2);
        assert!(definitions.find("border-style").is_some());
    }

    #[test]
    fn test_parser_one_to_four() {
        let _prop = PropertyDefinition {
            name: "border-style".to_string(),
            expanded_properties: vec![
                "border-top-style".to_string(),
                "border-right-style".to_string(),
                "border-bottom-style".to_string(),
                "border-left-style".to_string(),
            ],
            syntax: CssSyntax::new("".into()).compile().expect("Could not compile syntax"),
            inherits: false,
            initial_value: CssValue::String("thick".to_string()),
        };

        // let values = vec![
        //     CssValue::String("solid".to_string())
        // ];
        // assert_eq!(
        //     prop.parser_one_to_four(&values).unwrap(),
        //     vec![
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("solid".to_string()),
        //     ]
        // );

        // let values = vec![
        //     CssValue::String("solid".to_string()),
        //     CssValue::String("dashed".to_string())
        // ];
        // assert_eq!(
        //     prop.parser_one_to_four(&values).unwrap(),
        //     vec![
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("dashed".to_string()),
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("dashed".to_string()),
        //     ]
        // );

        // let values = vec![
        //     CssValue::String("solid".to_string()),
        //     CssValue::String("dashed".to_string()),
        //     CssValue::String("thick".to_string()),
        // ];
        // assert_eq!(
        //     prop.parser_one_to_four(&values).unwrap(),
        //     vec![
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("dashed".to_string()),
        //         CssValue::String("thick".to_string()),
        //         CssValue::String("dashed".to_string()),
        //     ]
        // );

        // let values = vec![
        //     CssValue::String("solid".to_string()),
        //     CssValue::String("dashed".to_string()),
        //     CssValue::String("thick".to_string()),
        //     CssValue::String("groove".to_string()),
        // ];
        // assert_eq!(
        //     prop.parser_one_to_four(&values).unwrap(),
        //     vec![
        //         CssValue::String("solid".to_string()),
        //         CssValue::String("dashed".to_string()),
        //         CssValue::String("thick".to_string()),
        //         CssValue::String("groove".to_string()),
        //     ]
        // );
    }


    #[test]
    fn test_azimuth() {
        let definitions = parse_definition_files().definitions;
        let def = definitions.get("azimuth").unwrap();

        assert!(def.matches(&CssValue::Unit(361.0, "deg".into())).is_none());
        // assert!(def.matches_str("-361deg").is_none());
        //
        // assert!(def.matches_str("1.570796326794897rad").is_some());
        // assert!(def.matches_str("0").is_some());
        // assert!(def.matches_str("360deg").is_some());
        // assert!(def.matches_str("36grad").is_some());
        // assert!(def.matches_str("2grad").is_some());
        // assert!(def.matches_str("-360deg").is_some());
        // assert!(def.matches_str("leftside").is_some());
        // assert!(def.matches_str("left-side").is_some());
        // assert!(def.matches_str("left").is_some());
        // assert!(def.matches_str("center").is_some());
        // assert!(def.matches_str("rightwards").is_some());
        // assert!(def.matches_str("behind far-right").is_some());
        // assert!(def.matches_str("behind").is_some());
        //
        //
        // assert!(def.matches(&CssValue::parse_str("361deg").unwrap()).is_none());
        // assert!(def.matches(&CssValue::parse_str("-361deg").unwrap()).is_none());
        // assert!(def.matches(&CssValue::parse_str("incorrect").unwrap()).is_none());
        // assert!(def.matches(&CssValue::parse_str("foobar").unwrap()).is_none());
        // assert!(def.matches(&CssValue::parse_str("").unwrap()).is_none());
        //
        // assert!(def.matches(&CssValue::parse_str("1.570796326794897rad").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("0").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("360deg").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("36grad").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("2grad").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("-360deg").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("leftside").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("left-side").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("left").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("center").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("rightwards").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("behind far-right").unwrap()).is_some());
        // assert!(def.matches(&CssValue::parse_str("behind").unwrap()).is_some());
    }
}
