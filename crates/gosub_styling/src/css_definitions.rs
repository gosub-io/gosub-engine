use gosub_css3::stylesheet::CssValue;
use memoize::memoize;
use std::collections::HashMap;
use log::warn;
use crate::syntax::{CssSyntax, Group, GroupCombinators, SyntaxComponent, SyntaxComponentMultiplier, SyntaxComponentType};
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
    /// Initial value of the property, if any
    initial_value: Option<CssValue>,
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

    pub fn has_initial_value(self) -> bool {
        self.initial_value.is_some()
    }

    pub fn initial_value(self) -> CssValue {
        self.initial_value.clone().unwrap_or(CssValue::None)
    }

    /// Matches a list of values against the current definition
    pub fn matches(self, value: &CssValue) -> Option<CssValue> {
        self.syntax.matches(value)
    }

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

    pub fn update_typedef(&mut self, name: &str, typedef: CssPropertyTypeDef) {
        self.typedefs.insert(name.to_string(), typedef);
    }

    pub fn find_scalar(&self, name: &str) -> Option<SyntaxComponent> {
        // println!("Finding scalar {:?}", name);
        let scalars = vec![
            "number",
            "integer",
            "percentage",
            "dashed-ident",
            "custom-ident",
            "ident",
            "repeat()",
            "attr()",
            "url",
            "uri",
            "named-color",
            "system-color",
            "unit()",
            "string",
            "tech()",
            "length",
            "reversed()"
        ];

        if scalars.contains(&name) {
            return Some(SyntaxComponent::new(SyntaxComponentType::Scalar(name.to_string()), SyntaxComponentMultiplier::Once));
        }

        None
    }

    pub fn find(&self, name: &str) -> Option<CssPropertyTypeDef> {
        // println!("Finding typedef {:?}", name);
        let names = vec![
            name.to_string(),
            format!("<{}>", name),
            format!("{}()", name),
        ];

        for name in names {
            if let Some(typedef) = self.typedefs.get(&name) {
                return Some(typedef.clone());
            }
        }

        None
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

    pub fn get_keys(self) -> Vec<String> {
        let mut keys = vec![];
        for (key, _) in self.typedefs.iter() {
            keys.push(key.clone());
        }
        keys
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
                // warn!("Could not compile syntax for typedef {:?}: {:?}", name, e);
                panic!("Could not compile syntax for typedef {:?}: {:?}", name, e);
            }
        }
    }

    let mut typedefs = CssPropertyTypeDefs { typedefs };

    // Resolve all typedefs since we now have loaded them all
    typedef_resolve_all(&mut typedefs);
    typedefs
}

/// Iterate all the typedefs and resolve any typedefs that are used in the syntax. After this call
/// no more typedefs should exist in the syntax.
fn typedef_resolve_all(typedefs: &mut CssPropertyTypeDefs) {
    for name in <CssPropertyTypeDefs as Clone>::clone(&typedefs).get_keys() {
        typedef_resolve(typedefs, &name);
    }
}

fn typedef_resolve_group(typedefs: &mut CssPropertyTypeDefs, group: &Group) -> Group {
    // println!("Resolving group {:?}", group);

    let mut resolved_group = Group{
        combinator: group.combinator.clone(),
        components: vec![],
    };

    for component in &group.components {
        match &component.type_ {
            SyntaxComponentType::TypeDefinition(name, _, _) => {
                // Is the type definition a scalar?
                if let Some(scalar) = typedefs.find_scalar(name) {
                    resolved_group.components.push(scalar);

                    continue;
                }

                if let Some(_typedef) = typedefs.find(name) {
                    // Resolve the typedef (if it's not already resolved and will take care of recursive typedefs)
                    typedef_resolve(typedefs, name);
                    resolved_group.components.push(typedefs.find(name).expect("Could not find typedef").syntax.components[0].clone());

                    continue;
                }

                dbg!(&name);
                dbg!(&typedefs.clone().get_keys());
                dbg!(&typedefs.typedefs.len());
                panic!("Reference to typedef {:?} found. But it's not defined", name);
            }
            SyntaxComponentType::Group(group) => {
                resolved_group.components.push(SyntaxComponent::new(SyntaxComponentType::Group(Group{
                    combinator: group.combinator.clone(),
                    components: typedef_resolve_group(typedefs, group).components.clone(),
                }), SyntaxComponentMultiplier::Once));
            },
            _ => {
                resolved_group.components.push(component.clone());
            }
        }
    }

    resolved_group
}

fn typedef_resolve_syntaxtree(typedefs: &mut CssPropertyTypeDefs, syntax_tree: CssSyntaxTree) -> CssSyntaxTree {
    // println!("Resolving syntax tree");

    let mut resolved_components = vec![];

    for component in &syntax_tree.components {
        // println!("Resolving component");

        // Resolve each component that needs resolving, or just return the component as-is.
        // If a component is a group, we need to resolve all its components first.
        match &component.type_ {
            // Resolve type definition
            SyntaxComponentType::TypeDefinition(name, _, _) => {

                // Is the type definition a scalar?
                if let Some(scalar) = typedefs.find_scalar(name) {
                    resolved_components.push(scalar);

                    continue;
                }

                if let Some(_typedef) = typedefs.find(name) {
                    // Resolve the typedef (if it's not already resolved and will take care of recursive typedefs)
                    typedef_resolve(typedefs, name);
                    resolved_components.push(typedefs.find(name).expect("Could not find typedef").syntax.components[0].clone());

                    continue;
                }

                panic!("Reference to typedef {:?} found. But it's not defined", name);
            }
            SyntaxComponentType::Group(group) => {
                let resolved_group = typedef_resolve_group(typedefs, group);
                resolved_components.push(SyntaxComponent::new(
                    SyntaxComponentType::Group(Group{
                        combinator: GroupCombinators::Juxtaposition,
                        components: resolved_group.components,
                    }),
                    SyntaxComponentMultiplier::Once
                ));
            },
            _ => {
                // No need to resolve this component, just add it as-is
                resolved_components.push(component.clone());
            }
        }
    }

    return CssSyntaxTree {
        components: resolved_components,
    };
}

/// Resolves a single typedef by recursively resolving all its components
fn typedef_resolve(typedefs: &mut CssPropertyTypeDefs, name: &str) {
    let mut typedef = typedefs.find(name).expect("Could not find typedef");
    typedef.syntax = typedef_resolve_syntaxtree(typedefs, typedef.syntax);
    typedefs.update_typedef(name, typedef);
}

/// Parses the JSON input into a CSS property definitions structure
fn parse_definition_file_internal(json: serde_json::Value, mut typedefs: CssPropertyTypeDefs) -> CssPropertyDefinitions {
    let mut definitions = HashMap::new();

    let entries = json.as_array().unwrap();
    for entry in entries {
        let name = entry["name"].as_str().unwrap().to_string();

        let mut expanded_properties = vec![];
        let mut syntax = CssSyntaxTree::new(vec![]);
        let mut inherits: bool = false;
        let mut initial_value: Option<CssValue> = None;

        if let Some(value) = entry["expanded_properties"].as_array() {
            expanded_properties = value
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
        }
        if let Some(value) = entry["syntax"].as_str() {
            if let Ok(ast) = CssSyntax::new(value).compile() {
                syntax = typedef_resolve_syntaxtree(&mut typedefs, ast.clone());
            } else {
                warn!("Could not compile syntax {:?}", entry);
                // panic!("Could not compile syntax {:?}", entry);
            }
        }
        if let Some(value) = entry["inherits"].as_bool() {
            inherits = value;
        }

        if let Some(value) = initial_value.clone() {
            // Can't have a syntax AND expanded properties
            if !expanded_properties.is_empty() {
                panic!(
                    "Expanded properties and initial value are mutually exclusive {:?}",
                    entry
                );
            }

            // If we have an initial value, make sure it matches the given syntax
            match syntax.matches(&value) {
                Some(v) => initial_value = Some(v.clone()),
                None => {
                    warn!("Cannot validate initial value {:?} against syntax for property {}", entry, name);
                }
            }
        }

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

    macro_rules! assert_none {
        ($e:expr) => {
            assert!($e.is_none());
        };
    }

    macro_rules! assert_some {
        ($e:expr) => {
            assert!($e.is_some());
        };
    }

    #[test]
    fn test_parse_definition_file() {
        let definitions = parse_definition_files();
        assert_eq!(definitions.len(), 115);
    }

    #[test]
    fn test_prop_def() {
        let definitions = parse_definition_files();

        let prop = definitions.find("color").unwrap();
        assert_some!(prop.clone().matches(&CssValue::Color(RgbColor::from("#ff0000"))));
        assert_none!(prop.clone().matches(&CssValue::Number(42.0)));

        let prop = definitions.find("border").unwrap();
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::Color(RgbColor::from("black")),
            CssValue::String("solid".into()),
            CssValue::Unit(1.0, "px".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
            CssValue::Unit(1.0, "px".into()),
        ])));
        assert_some!(prop.clone().matches(&CssValue::Unit(1.0, "px".into())));
        assert_some!(prop.clone().matches(&CssValue::String("solid".into())));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
        ])));
        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
        ])));
        assert_some!(prop.clone().matches(&CssValue::String("solid".into())));
    }

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
                initial_value: None,
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
                initial_value: Some(CssValue::String("thick".to_string())),
            },
        );

        assert_eq!(definitions.len(), 2);
        assert!(definitions.find("border-style").is_some());
    }

    #[test]
    fn test_azimuth() {
        let definitions = parse_definition_files().definitions;
        let def = definitions.get("azimuth").unwrap();

        assert_some!(def.clone().matches(&CssValue::Unit(361.0, "deg".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(361.0, "deg".into())));

        assert_none!(def.clone().matches(&CssValue::Unit(20.0, "blaat".into())));

        assert_some!(def.clone().matches(&CssValue::Unit(std::f32::consts::FRAC_PI_2, "rad".into())));
        assert_some!(def.clone().matches(&CssValue::Number(0.0)));

        assert_some!(def.clone().matches(&CssValue::Unit(360.0, "deg".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(360.0, "grad".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(2.0, "grad".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(-360.0, "grad".into())));
        assert_some!(def.clone().matches(&CssValue::String("leftside".into())));
        assert_some!(def.clone().matches(&CssValue::String("left-side".into())));
        assert_some!(def.clone().matches(&CssValue::String("left".into())));
        assert_some!(def.clone().matches(&CssValue::String("center".into())));
        assert_some!(def.clone().matches(&CssValue::String("rightwards".into())));
        assert_some!(def.clone().matches(&CssValue::List(vec!(
            CssValue::String("behind".into()),
            CssValue::String("far-right".into()),
        ))));
        assert_some!(def.clone().matches(&CssValue::String("behind".into())));
    }
}
