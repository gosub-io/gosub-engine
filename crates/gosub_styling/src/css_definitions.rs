use std::collections::HashMap;
use log::warn;
use memoize::memoize;
use gosub_css3::stylesheet::CssValue;
use crate::syntax::{CssSyntax, SyntaxComponent};
use crate::syntax::GroupCombinators::Juxtaposition;
use crate::syntax_matcher::CssSyntaxTree;

/// List of elements that are built-in data types in the CSS specification
#[allow(dead_code)]
const BUILTIN_DATA_TYPES: [&str; 24] = [
    "anchor-element",
    "angle",
    "coord-box",
    "custom-ident",
    "dashed-ident",
    "declaration-value",
    "hex-color",
    "inset-area",
    "integer",
    "length",
    "number",
    "offset-path",
    "palette-identifier",
    "percentage",
    "resolution",
    "single-animation-composition",
    "string",
    "time",
    "try-size",
    "try-tactic",
    "url",
    "white-space-trim",
    "x",
    "y",
];

/// A CSS property definition including its type and initial value and optional expanded values if it's a shorthand property
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PropertyDefinition {
    /// Name of the property (ie: color, background etc)
    name: String,
    /// List of expanded (computed) properties if this property is a shorthand property
    computed: Vec<String>,
    /// Syntax tree of the property. This is a tree that describes the valid values for this property.
    syntax: CssSyntaxTree,
    /// True when the property inherits from parent nodes if not set
    inherited: bool,
    /// Initial value of the property, if any
    initial_value: Option<CssValue>,
    /// URL to MDN documentation for this property
    mdn_url: String,
    // True when this element is resolved
    resolved: bool
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SyntaxDefinition {
    /// Name of the syntax definition
    name: String,
    /// Actual syntax
    syntax: CssSyntaxTree,
    /// True when the element has already been resolved
    resolved: bool
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FunctionDefinition {
    /// Name of the function
    name: String,
    /// Compiled syntax tree
    syntax: CssSyntaxTree,
    /// URL to MDN documentation for this function
    mdn_url: String
}

impl PropertyDefinition {
    pub fn name(self) -> String {
        self.name.clone()
    }

    pub fn expanded_properties(self) -> Vec<String> {
        self.computed.clone()
    }

    pub fn syntax(self) -> CssSyntaxTree {
        self.syntax
    }

    pub fn inherited(self) -> bool {
        self.inherited
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

#[derive(Debug, Clone)]
pub struct CssDefinitions {
    pub properties: HashMap<String, PropertyDefinition>,
    pub functions: HashMap<String, FunctionDefinition>,
    pub syntax: HashMap<String, SyntaxDefinition>,
}

impl CssDefinitions {
    pub fn new() -> Self {
        CssDefinitions {
            properties: HashMap::new(),
            functions: HashMap::new(),
            syntax: HashMap::new(),
        }
    }

    /// Load the CSS definitions resource files
    pub fn load() {
        parse_mdn_definition_files();
    }

    /// Add a new property definition
    pub fn add_property(&mut self, name: &str, property: PropertyDefinition) {
        self.properties.insert(name.to_string(), property);
    }

    /// Add a new syntax definition
    pub fn add_syntax(&mut self, name: &str, syntax: SyntaxDefinition) {
        self.syntax.insert(name.to_string(), syntax);
    }

    /// Add a new function definition
    pub fn add_function(&mut self, name: &str, function: FunctionDefinition) {
        self.functions.insert(name.to_string(), function);
    }

    /// Find a specific property
    pub fn find_property(&self, name: &str) -> Option<PropertyDefinition> {
        self.properties.get(name).cloned()
    }

    /// Returns the property definitions
    pub fn get_properties(self) -> HashMap<String, PropertyDefinition> {
        self.properties.clone()
    }

    /// Returns the length of the property definitions
    pub fn len(&self) -> usize {
        self.properties.len()
    }

    /// Returns true when the properties definitions are empty
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty()
    }
}

impl CssDefinitions {

    /// Resolves all elements in the definitions, syntax and functions
    pub fn resolve(&mut self){
        let mut names = self.properties.keys().cloned().collect::<Vec<String>>();
        names.sort();

        for name in names {
            self.resolve_property(&name);
        }
    }

    /// Resolves a property definition (recursive)
    fn resolve_property(&mut self, name: &str) -> PropertyDefinition {
        // println!("Resolving property: {:?}", name);

        let mut element = self.find_property(name).expect("Could not find property definition");
        if !element.resolved {
            // Resolve if not resolved already
            let mut resolved_components = vec![];
            for component in &element.syntax.components {
                // println!("  resolving component in property: {:?}", component);
                let component = self.resolve_component(component, name);
                resolved_components.push(component);
            }

            // update element in properties
            element.syntax.components = resolved_components;
            element.resolved = true;
            self.add_property(name, element.clone());
        }

        element
    }

    #[allow(dead_code)]
    fn resolve_component(&mut self, component: &SyntaxComponent, prop_name: &str) -> SyntaxComponent {
        // println!("resolve_component: {:?}", component);

        match component {
            SyntaxComponent::Definition { datatype, multiplier, .. } => {
                // println!("Resolving definition {:?}", datatype);

                // Find the datatype in the syntax definitions
                // println!("syntax check for datatype: {:?}", datatype);
                if let Some(syntax_element) = self.syntax.get(datatype) {
                    // println!("definition is a syntax element");

                    let mut syntax_element = syntax_element.clone();
                    if !syntax_element.resolved {
                        syntax_element.syntax = self.resolve_syntax(&syntax_element.syntax, prop_name);
                        syntax_element.resolved = true;
                    }

                    return SyntaxComponent::Group {
                        components: syntax_element.syntax.components.clone(),
                        combinator: Juxtaposition,
                        multiplier: multiplier.clone(),
                    };
                }

                // Don't resolve in properties when the datatype is the same as the propertyname (for instance: inset-area)
                if datatype != prop_name {
                    // Find the datatype in the properties definitions
                    if let Some(property_element) = self.properties.get(datatype) {
                        // println!("definition is a property element");

                        let resolved_prop = if !property_element.resolved {
                            self.resolve_property(&datatype)
                        } else {
                            property_element.clone()
                        };

                        // If the resolved syntax is just a single element (be it a group, or a single element),
                        // return that component.
                        if resolved_prop.syntax.components.len() == 1 {
                            return resolved_prop.syntax.components[0].clone();
                        }

                        // Otherwise, we return a group with the components
                        return SyntaxComponent::Group {
                            components: resolved_prop.syntax.components.clone(),
                            combinator: Juxtaposition,
                            multiplier: multiplier.clone(),
                        };
                    }
                }

                // Finally, check if the data type is a built-in datatype
                if BUILTIN_DATA_TYPES.contains(&datatype.as_str()) {
                    // println!("definition is a built-in element");

                    // Ok, it's a built-in datatype, convert it to a built-in type
                    return SyntaxComponent::Builtin {
                        datatype: datatype.clone(),
                        multiplier: multiplier.clone()
                    };
                }

                panic!("Unknown datatype encountered: {:?}", datatype);
            }
            SyntaxComponent::Group { components, combinator, multiplier } => {
                // Resolve each component from the group
                let mut resolved_components = vec![];
                // Resolve all elements in the group
                for component in components {
                    // println!("resolving group component: {:?}", component);
                    resolved_components.push(self.resolve_component(component, prop_name));
                }

                return SyntaxComponent::Group{
                    components: resolved_components,
                    combinator: combinator.clone(),
                    multiplier: multiplier.clone(),
                }
            }
            _ => {
                // This component does not need any resolving
                return component.clone()
            }
        }

    }

    fn resolve_syntax(&mut self, syntax: &CssSyntaxTree, prop_name: &str) -> CssSyntaxTree {
        let mut resolved_components = vec![];

        for component in &syntax.components {
            resolved_components.push(self.resolve_component(component, prop_name));
        }

        CssSyntaxTree {
            components: resolved_components
        }
    }
}

/// Parses the internal CSS definition file
#[memoize]
pub fn parse_mdn_definition_files() -> CssDefinitions {
    // First, parse all functions so we can use them in the properties and syntax
    let contents = include_str!("../resources/mdn_css_functions.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let functions = parse_mdn_functions_file(json);

    // parse all syntax so we can use them in the properties
    let contents = include_str!("../resources/mdn_css_syntax.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let syntax = parse_mdn_syntax_file(json);

    // Parse property definitions
    let contents = include_str!("../resources/mdn_css_properties.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let properties = parse_mdn_property_file(json);

    let mut definitions = CssDefinitions{
        properties,
        functions,
        syntax
    };

    // Resolve all syntax and functions inside the definitions
    definitions.resolve();

    definitions
}

/// Main function to return the definitions. THis will automatically load the definition files
/// and caches them if needed.
pub fn get_mdn_css_definitions() -> CssDefinitions {
    parse_mdn_definition_files()
}

/// Parses a function JSON import file
fn parse_mdn_functions_file(json: serde_json::Value) -> HashMap<String, FunctionDefinition> {
    let mut functions = HashMap::new();

    for obj in json.as_array().unwrap() {
        let syntax = obj.get("syntax").unwrap().as_str().unwrap();
        let syntax = CssSyntax::new(syntax).compile().expect("Could not compile syntax");
        functions.insert(
            obj.get("name").unwrap().to_string(),
            FunctionDefinition {
                name: obj.get("name").unwrap().clone().to_string(),
                syntax,
                mdn_url: obj.get("mdn_url").unwrap().to_string(),
            },
        );
    }

    functions
}

/// Parses a syntax JSON import file
fn parse_mdn_syntax_file(json: serde_json::Value) -> HashMap<String, SyntaxDefinition> {
    let mut syntaxes = HashMap::new();

    let entries = json.as_object().unwrap();
    for (name, entry) in entries.iter() {
        match CssSyntax::new(entry.as_str().unwrap()).compile() {
            Ok(ast) => {
                syntaxes.insert(
                    name.clone(),
                    SyntaxDefinition {
                        name: name.clone(),
                        syntax: ast.clone(),
                        resolved: false,
                    },
                );
            }
            Err(e) => {
                log::warn!("Could not compile syntax for syntax {:?}: {:?}", name, e);
            }
        }
    }

    // Resolve all typedefs since we now have loaded them all
    // typedef_resolve_all(&mut typedefs);
    syntaxes
}

/// Parses the JSON input into a CSS property definitions structure
fn parse_mdn_property_file(json: serde_json::Value) -> HashMap<String, PropertyDefinition> {
    let mut properties = HashMap::new();

    for obj in json.as_array().unwrap() {
        let name = obj["name"].as_str().unwrap().to_string();

        // Compile syntax
        let syntax = obj.get("syntax").unwrap().as_str().unwrap();
        let syntax = CssSyntax::new(syntax).compile().expect(format!("Could not compile syntax: {:?}", syntax).as_str());

        //
        let computed = if obj["computed"].is_array() {
            obj["computed"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect()
        } else if obj["computed"].is_string() {
            vec![obj["computed"].as_str().unwrap().to_string()]
        } else {
            warn!("Computed property is not a string or array {:?}", obj);
            vec![]
        };

        let initial_value = if obj["initial_value"].is_array() {
            warn!("Initial value is an array, not supported {:?}", obj);
            // obj["initial_value"]
            //     .as_array()
            //     .unwrap()
            //     .iter()
            //     .map(|v| CssValue::from(v))
            //     .collect()
            None
        } else if obj["initial_value"].is_string() {
            match CssValue::parse_str(obj["initial_value"].as_str().unwrap()) {
                Ok(value) => Some(value),
                Err(e) => {
                    warn!("Could not parse initial value: {:?}", e);
                    None
                }
            }
        } else {
            warn!("Initial value is not a string or array {:?}", obj);
            None
        };

        properties.insert(
            name.clone(),
            PropertyDefinition {
                name: name.clone(),
                syntax,
                computed,
                initial_value,
                inherited: obj["inherited"].as_bool().unwrap(),
                mdn_url: obj["mdn_url"].as_str().unwrap().to_string(),
                resolved: false,
            },
        );
    }

    properties
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_css3::colors::RgbColor;

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
        let definitions = parse_mdn_definition_files();
        assert_eq!(definitions.len(), 563);
    }

    #[test]
    fn test_prop_border() {
        let definitions = parse_mdn_definition_files();
        let prop = definitions.find_property("border").unwrap();

        assert_some!(prop.clone().matches(&CssValue::List(vec![
            CssValue::Unit(1.0, "px".into()),
            CssValue::String("solid".into()),
            CssValue::Color(RgbColor::from("black")),
        ])));

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
        assert_none!(prop.clone().matches(&CssValue::String("not-solid".into())));
        assert_none!(prop.clone().matches(&CssValue::List(vec![
            CssValue::String("solid".into()),
            CssValue::String("solid".into()),
            CssValue::Unit(1.0, "px".into()),
        ])));
    }

    #[test]
    fn test_property_definitions() {
        let mut definitions = CssDefinitions::new();
        definitions.add_property(
            "color",
            PropertyDefinition {
                name: "color".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("color()")
                    .compile()
                    .expect("Could not compile syntax"),
                inherited: false,
                initial_value: None,
                mdn_url: "".to_string(),
                resolved: false,
            },
        );

        assert_eq!(definitions.len(), 1);
        assert!(definitions.find_property("color").is_some());
        assert!(definitions.find_property("border-top-style").is_none());

        definitions.add_property(
            "border-style",
            PropertyDefinition {
                name: "border-style".to_string(),
                computed: vec![
                    "border-top-style".to_string(),
                    "border-right-style".to_string(),
                    "border-bottom-style".to_string(),
                    "border-left-style".to_string(),
                ],
                syntax: CssSyntax::new("")
                    .compile()
                    .expect("Could not compile syntax"),
                inherited: false,
                initial_value: Some(CssValue::String("thick".to_string())),
                mdn_url: "".to_string(),
                resolved: false,
            },
        );

        assert_eq!(definitions.len(), 2);
        assert!(definitions.find_property("border-style").is_some());
    }

    #[test]
    fn test_azimuth() {
        let definitions = parse_mdn_definition_files();
        let def = definitions.find_property("azimuth").unwrap();

        assert_some!(def.clone().matches(&CssValue::Unit(361.0, "deg".into())));

        assert_none!(def.clone().matches(&CssValue::Unit(20.0, "blaat".into())));

        assert_some!(def
            .clone()
            .matches(&CssValue::Unit(std::f32::consts::FRAC_PI_2, "rad".into())));
        assert_some!(def.clone().matches(&CssValue::Zero));

        assert_some!(def.clone().matches(&CssValue::Unit(360.0, "deg".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(360.0, "grad".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(2.0, "grad".into())));
        assert_some!(def.clone().matches(&CssValue::Unit(-360.0, "grad".into())));

        assert_none!(def.clone().matches(&CssValue::String("leftside".into())));

        assert_some!(def.clone().matches(&CssValue::String("left-side".into())));
        assert_some!(def.clone().matches(&CssValue::String("left".into())));
        assert_some!(def.clone().matches(&CssValue::String("center".into())));
        assert_some!(def.clone().matches(&CssValue::String("rightwards".into())));
        assert_some!(def.clone().matches(&CssValue::List(vec!(
            CssValue::String("far-right".into()),
            CssValue::String("behind".into()),
        ))));
        assert_some!(def.clone().matches(&CssValue::String("behind".into())));
    }

    #[test]
    fn test_background_color() {
        let definitions = parse_mdn_definition_files();
        let def = definitions.find_property("background-color").unwrap();

        // assert_some!(def.clone().matches(&CssValue::Inherit));
        assert_some!(def.clone().matches(&CssValue::String("transparent".into())));

        assert_some!(def.clone().matches(&CssValue::String("red".into())));
        // System colors
        // assert_some!(def.clone().matches(&CssValue::String("Canvas".into())));
        // assert_some!(def.clone().matches(&CssValue::String("CanvasText".into())));
        // assert_some!(def.clone().matches(&CssValue::String("CanvasText".into())));
        assert_some!(def.clone().matches(&CssValue::String("Menu".into())));

        assert_some!(def.clone().matches(&CssValue::String("blue".into())));
        assert_some!(def
            .clone()
            .matches(&CssValue::Color(RgbColor::from("#ff0000"))));
        assert_some!(def
            .clone()
            .matches(&CssValue::String("rebeccapurple".into())));

        assert_none!(def
            .clone()
            .matches(&CssValue::String("thiscolordoesnotexist".into())));
    }

    #[test]
    fn test_background_attachments() {
        let definitions = parse_mdn_definition_files();
        let def = definitions.find_property("background-attachment").unwrap();

        // assert_some!(def.clone().matches(&CssValue::Inherit));
        assert_some!(def.clone().matches(&CssValue::String("scroll".into())));
        assert_some!(def.clone().matches(&CssValue::String("fixed".into())));

        assert_none!(def.clone().matches(&CssValue::String("incorrect".into())));
        assert_none!(def
            .clone()
            .matches(&CssValue::String("rebeccapurple".into())));
        assert_none!(def.clone().matches(&CssValue::Zero));
    }

    #[test]
    fn test_background_position() {
        let definitions = parse_mdn_definition_files();
        let def = definitions.find_property("background-position").unwrap();

        // assert_none!(def.clone().matches(&CssValue::String("scroll".into())));
        // assert_none!(def.clone().matches(&CssValue::String("fixed".into())));
        // assert_none!(def.clone().matches(&CssValue::String("incorrect".into())));
        // assert_none!(def
        //     .clone()
        //     .matches(&CssValue::String("rebeccapurple".into())));
        //
        // assert_some!(def.clone().matches(&CssValue::Percentage(0.0)));
        // assert_some!(def.clone().matches(&CssValue::Zero));
        // assert_some!(def.clone().matches(&CssValue::Unit(12.34, "px".into())));
        // assert_none!(def.clone().matches(&CssValue::Number(12.34)));


        /*
        left
        center
        right
        top
        bottom
        10px
        10%

        left top
        left center
        left bottom
        left 10px
        left 20%
        center 20%
        center center
        10% center
        10% 20px
        10% 20%
        10px 20px
        10px 20%
        center left
        center left 20%
        center right 30px

        left 20% right 30px
        left right
         */




        // background-position: left 10px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Unit(10.0, "px".into()),
        ])));

        // background-position: left 10px top 20px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Unit(10.0, "px".into()),
            CssValue::String("top".into()),
            CssValue::Unit(20.0, "px".into()),
        ])));

        // background-position: right 15% bottom 5%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("right".into()),
            CssValue::Percentage(15.0),
            CssValue::String("bottom".into()),
            CssValue::Percentage(5.0),
        ])));

        // background-position: center center;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("center".into()),
            CssValue::String("center".into()),
        ])));

        // background-position: 75% 50%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(75.0),
            CssValue::Percentage(50.0),
        ])));

        // background-position: 75%;
        assert_some!(def
            .clone()
            .matches(&CssValue::List(vec![CssValue::Percentage(75.0),])));

        // background-position: top 10px center;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("top".into()),
            CssValue::Unit(10.0, "px".into()),
            CssValue::String("center".into()),
        ])));

        // background-position: bottom 20px right 30px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("bottom".into()),
            CssValue::Unit(20.0, "px".into()),
            CssValue::String("right".into()),
            CssValue::Unit(30.0, "px".into()),
        ])));

        // background-position: 20% 80%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(20.0),
            CssValue::Percentage(80.0),
        ])));

        // background-position: left 5px bottom 15px, right 10px top 20px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Unit(5.0, "px".into()),
            CssValue::String("bottom".into()),
            CssValue::Unit(15.0, "px".into()),
            CssValue::String("right".into()),
            CssValue::Unit(10.0, "px".into()),
            CssValue::String("top".into()),
            CssValue::Unit(20.0, "px".into()),
        ])));

        // background-position: center top 35px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("center".into()),
            CssValue::String("top".into()),
            CssValue::Unit(35.0, "px".into()),
        ])));

        // background-position: left 45% bottom 25%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Percentage(45.0),
            CssValue::String("bottom".into()),
            CssValue::Percentage(25.0),
        ])));

        // background-position: right 10% top 50px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("right".into()),
            CssValue::Percentage(10.0),
            CssValue::String("top".into()),
            CssValue::Unit(50.0, "px".into()),
        ])));

        // background-position: 0% 0%, 100% 100%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(0.0),
            CssValue::Percentage(0.0),
            CssValue::Percentage(100.0),
            CssValue::Percentage(100.0),
        ])));

        // background-position: left top, right bottom;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::String("top".into()),
            CssValue::String("right".into()),
            CssValue::String("bottom".into()),
        ])));

        // background-position: 100% 0, 0 100%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(100.0),
            CssValue::Number(0.0),
            CssValue::Number(0.0),
            CssValue::Percentage(100.0),
        ])));

        // background-position: left 25px bottom, center top;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("left".into()),
            CssValue::Unit(25.0, "px".into()),
            CssValue::String("bottom".into()),
            CssValue::String("center".into()),
            CssValue::String("top".into()),
        ])));

        // background-position: top 10% left 20%, bottom 10% right 20%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("top".into()),
            CssValue::Percentage(10.0),
            CssValue::String("left".into()),
            CssValue::Percentage(20.0),
            CssValue::String("bottom".into()),
            CssValue::Percentage(10.0),
            CssValue::String("right".into()),
            CssValue::Percentage(20.0),
        ])));

        // background-position: 10px 30px, 90% 10%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Unit(10.0, "px".into()),
            CssValue::Unit(30.0, "px".into()),
            CssValue::Percentage(90.0),
            CssValue::Percentage(10.0),
        ])));

        // background-position: top right, bottom left 15px;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("top".into()),
            CssValue::String("right".into()),
            CssValue::String("bottom".into()),
            CssValue::String("left".into()),
            CssValue::Unit(15.0, "px".into()),
        ])));

        // background-position: 50% 25%, 25% 75%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::Percentage(50.0),
            CssValue::Percentage(25.0),
            CssValue::Percentage(25.0),
            CssValue::Percentage(75.0),
        ])));

        // background-position: right 5% bottom 5%, left 5% top 5%;
        assert_some!(def.clone().matches(&CssValue::List(vec![
            CssValue::String("right".into()),
            CssValue::Percentage(5.0),
            CssValue::String("bottom".into()),
            CssValue::Percentage(5.0),
            CssValue::String("left".into()),
            CssValue::Percentage(5.0),
            CssValue::String("top".into()),
            CssValue::Percentage(5.0),
        ])));
    }
}
