use std::collections::HashMap;

use log::warn;

use gosub_css3::stylesheet::CssValue;

use crate::shorthands::{FixList, Shorthands};
use std::sync::LazyLock;

use crate::syntax::GroupCombinators::Juxtaposition;
use crate::syntax::{CssSyntax, SyntaxComponent};
use crate::syntax_matcher::CssSyntaxTree;

/// List of elements that are built-in data types in the CSS specification. These will be handled
/// by the syntax matcher as built-in types.
const BUILTIN_DATA_TYPES: [&str; 41] = [
    "absolute-size",
    "age",
    "angle",
    "basic-shape",
    "calc-size()",
    "counter-name",
    "counter-style-name",
    "custom-ident",
    "dashed-ident",
    "decibel",
    "feature-tag-value",
    "flex",
    "frequency",
    "gender",
    "hex-color",
    "id",
    "ident",
    "image-1D",
    "integer",
    "length",
    "number",
    "named-color",
    "semitones",
    "system-color",
    "outline-line-style",
    "palette-identifier",
    "percentage",
    "relative-size",
    "string",
    "target-name",
    "time",
    "timeline-range-name",
    "transform-function",
    "uri",
    "url-set",
    "url-token",
    "x",
    "y",
    "color()",
    "attr()",    //TODO: this is not a builtin!
    "element()", //TODO: this is not a builtin!
];

/// A CSS property definition including its type and initial value and optional expanded values if it's a shorthand property
#[derive(Debug, Clone)]
pub struct PropertyDefinition {
    /// Name of the property (ie: color, background etc)
    pub name: String,
    /// List of expanded (computed) properties if this property is a shorthand property
    pub computed: Vec<String>,
    /// Syntax tree of the property. This is a tree that describes the valid values for this property.
    pub syntax: CssSyntaxTree,
    /// True when the property inherits from parent nodes if not set
    pub inherited: bool,
    /// Initial value of the property, if any
    pub initial_value: Option<CssValue>,
    // True when this element is resolved
    pub resolved: bool,
    /// Shorthand resolver, used to expand computed values
    pub shorthands: Option<Shorthands>,
}

impl PropertyDefinition {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn expanded_properties(&self) -> Vec<String> {
        self.computed.clone()
    }

    pub fn syntax(&self) -> &CssSyntaxTree {
        &self.syntax
    }

    pub fn inherited(&self) -> bool {
        self.inherited
    }

    /// Returns true when this definition has an initial value
    pub fn has_initial_value(&self) -> bool {
        self.initial_value.is_some()
    }

    /// Returns the initial value
    pub fn initial_value(&self) -> CssValue {
        self.initial_value.clone().unwrap_or(CssValue::None)
    }

    /// Matches a list of values against the current definition
    pub fn matches(&self, input: &[CssValue]) -> bool {
        self.syntax.matches(input)
    }

    pub fn matches_and_shorthands(&self, input: &[CssValue], fix_list: &mut FixList) -> bool {
        if let Some(shorthands) = &self.shorthands {
            let resolver = shorthands.get_resolver(fix_list);

            self.syntax.matches_and_shorthands(input, resolver)
        } else {
            self.syntax.matches(input)
        }
    }

    pub fn check_expanded_properties(&self, _values: &[CssValue]) -> bool {
        // if values.len() != self.expanded_properties.len() {
        //     return false;
        // }
        //
        // for (i, value) in values.iter().enumerate() {
        //     let prop = self.expanded_properties.get(i).unwrap();
        //     let prop_def = parse_definition_file().find(prop).unwrap();
        //     if !prop_def.matches(&[value.clone()]) {
        //         return false;
        //     }
        // }

        true
    }

    pub fn is_shorthand(&self) -> bool {
        self.computed.len() > 1
    }
}

/// A syntax definition that can be used to resolve a property definition
#[derive(Debug, Clone)]
pub struct SyntaxDefinition {
    /// Actual syntax
    pub syntax: CssSyntaxTree,
    /// True when the element has already been resolved
    resolved: bool,
}

/// Defines a list of CSS properties and its syntax.
#[derive(Debug, Clone)]
pub struct CssDefinitions {
    // List of all resolved properties
    pub resolved_properties: HashMap<String, PropertyDefinition>,
    /// All defined properties
    pub properties: HashMap<String, PropertyDefinition>,
    /// List of syntax elements for resolving the properties
    pub syntax: HashMap<String, SyntaxDefinition>,
}

impl Default for CssDefinitions {
    fn default() -> Self {
        Self::new()
    }
}

impl CssDefinitions {
    pub fn new() -> Self {
        CssDefinitions {
            resolved_properties: HashMap::new(),
            properties: HashMap::new(),
            syntax: HashMap::new(),
        }
    }

    /// Load the CSS definitions resource files
    pub fn load() {
        let _ = CSS_DEFINITIONS.len();
    }

    /// Add a new property definition
    pub fn add_property(&mut self, name: &str, property: PropertyDefinition) {
        self.properties.insert(name.to_string(), property);
    }

    /// Add a new syntax definition
    pub fn add_syntax(&mut self, name: &str, syntax: SyntaxDefinition) {
        self.syntax.insert(name.to_string(), syntax);
    }

    /// Find a specific property
    pub fn find_property(&self, name: &str) -> Option<&PropertyDefinition> {
        self.resolved_properties.get(name)
    }

    /// Returns the length of the property definitions
    pub fn len(&self) -> usize {
        self.resolved_properties.len()
    }

    /// Returns true when the properties definitions are empty
    pub fn is_empty(&self) -> bool {
        self.resolved_properties.is_empty()
    }

    /// Resolves all elements in the definitions
    pub fn resolve(&mut self) {
        let mut names = self.properties.keys().cloned().collect::<Vec<String>>();
        names.sort();

        for name in names {
            self.resolve_property(&name);
        }
    }

    /// Resolves a property definition (recursively)
    fn resolve_property(&mut self, name: &str) {
        if self.resolved_properties.contains_key(name) {
            // Property already resolved
            return;
        }

        if !self.properties.contains_key(name) {
            // Property not found to resolve
            return;
        }

        // Resolve the element
        let mut element = self.properties.get_mut(name).unwrap().clone();
        let mut resolved_components = vec![];
        for component in &element.syntax.components {
            let component = self.resolve_component(component, name);
            resolved_components.push(component);
        }

        element.syntax.components = resolved_components;
        element.resolved = true;
        self.resolved_properties.insert(name.to_string(), element);
    }

    /// Resolve a syntax component
    pub fn resolve_component(
        &mut self,
        component: &SyntaxComponent,
        prop_name: &str,
    ) -> SyntaxComponent {
        match component {
            SyntaxComponent::Definition {
                datatype,
                multipliers,
                ..
            } => {
                // First step: Resolve by looking the definition up in the syntax defintions.
                if let Some(syntax_element) = self.syntax.get(datatype) {
                    let mut syntax_element = syntax_element.clone();
                    if !syntax_element.resolved {
                        syntax_element.syntax =
                            self.resolve_syntax(&syntax_element.syntax, prop_name);
                        syntax_element.resolved = true;
                        self.syntax.insert(datatype.clone(), syntax_element.clone());
                    }

                    return SyntaxComponent::Group {
                        components: syntax_element.syntax.components.clone(),
                        combinator: Juxtaposition,
                        multipliers: multipliers.clone(),
                    };
                }

                // Second step: Resolve by looking the definition up in the properties

                // Don't resolve in properties when the datatype is the same as the
                // property name (for instance: inset-area)
                if datatype != prop_name {
                    // This datatype is not resolved yet.
                    if !self.resolved_properties.contains_key(datatype) {
                        if let Some(property_element) = self.properties.get(datatype) {
                            let name = property_element.name.clone();
                            self.resolve_property(name.as_str());
                        }
                    }

                    if let Some(resolved_prop) = self.resolved_properties.get(datatype) {
                        // If the resolved syntax is just a single element (be it a group, or a single element),
                        // return that component.
                        if resolved_prop.syntax.components.len() == 1 {
                            let mut component = resolved_prop.syntax.components[0].clone();

                            component.update_multipliers(multipliers.clone());

                            return component;
                        }
                        // Otherwise, we return a group with the components
                        return SyntaxComponent::Group {
                            components: resolved_prop.syntax.components.clone(),
                            combinator: Juxtaposition,
                            multipliers: multipliers.clone(),
                        };
                    }
                }

                // Last step: check if the data type is a built-in datatype
                if BUILTIN_DATA_TYPES.contains(&datatype.as_str()) {
                    return SyntaxComponent::Builtin {
                        datatype: datatype.clone(),
                        multipliers: multipliers.clone(),
                    };
                }

                panic!("Unknown datatype encountered: {:?}", datatype);
            }
            SyntaxComponent::Group {
                components,
                combinator,
                multipliers,
            } => {
                // Resolve this group and return a new group with resolved components
                let mut resolved_components = vec![];
                for component in components {
                    resolved_components.push(self.resolve_component(component, prop_name));
                }

                SyntaxComponent::Group {
                    components: resolved_components,
                    combinator: combinator.clone(),
                    multipliers: multipliers.clone(),
                }
            }
            _ => {
                // This component does not need any resolving
                component.clone()
            }
        }
    }

    // Resolve all the components from a given syntax tree
    fn resolve_syntax(&mut self, syntax: &CssSyntaxTree, prop_name: &str) -> CssSyntaxTree {
        let mut resolved_components = vec![];

        for component in &syntax.components {
            resolved_components.push(self.resolve_component(component, prop_name));
        }

        CssSyntaxTree {
            components: resolved_components,
        }
    }
}

pub static CSS_DEFINITIONS: LazyLock<CssDefinitions, fn() -> CssDefinitions> =
    LazyLock::new(pars_definition_files);

/// Parses the internal CSS definition file
fn pars_definition_files() -> CssDefinitions {
    // parse all syntax, so we can use them in the properties
    let contents = include_str!("../resources/definitions/definitions_values.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let syntax = parse_syntax_file(json);

    // Parse property definitions
    let contents = include_str!("../resources/definitions/definitions_properties.json");
    let json: serde_json::Value =
        serde_json::from_str(contents).expect("JSON was not well-formatted");
    let properties = parse_property_file(json);

    // Create definition structure, and resolve all definitions
    let mut definitions = CssDefinitions {
        resolved_properties: HashMap::new(),
        properties,
        syntax,
    };

    definitions.index_shorthands();
    definitions.resolve();

    definitions
}

/// Main function to return the definitions. This will automatically load the definition files
/// and caches them if needed.
pub fn get_css_definitions() -> &'static CssDefinitions {
    &CSS_DEFINITIONS
}

/// Parses a syntax JSON import file
fn parse_syntax_file(json: serde_json::Value) -> HashMap<String, SyntaxDefinition> {
    let mut syntaxes = HashMap::new();

    let entries = json.as_array().unwrap();
    for entry in entries.iter() {
        match CssSyntax::new(entry.get("syntax").unwrap().as_str().unwrap()).compile() {
            Ok(ast) => {
                let mut name = entry.get("name").unwrap().to_string();

                if name.starts_with('"') {
                    name = name[1..].to_string();
                }

                if name.starts_with('<') {
                    name = name[1..].to_string();
                }

                if name.ends_with('"') {
                    name.pop();
                }

                if name.ends_with('>') {
                    name.pop();
                }

                syntaxes.insert(
                    name.clone(),
                    SyntaxDefinition {
                        // name,
                        syntax: ast.clone(),
                        resolved: false,
                    },
                );
            }
            Err(e) => {
                log::warn!(
                    "Could not compile syntax for syntax {:?}: {:?}",
                    entry.get("name").unwrap().to_string(),
                    e
                );
            }
        }
    }

    // Resolve all typedefs since we now have loaded them all
    // typedef_resolve_all(&mut typedefs);
    syntaxes
}

/// Parses the JSON input into a CSS property definitions structure
fn parse_property_file(json: serde_json::Value) -> HashMap<String, PropertyDefinition> {
    let mut properties = HashMap::new();

    for obj in json.as_array().unwrap() {
        let name = obj["name"].as_str().unwrap().to_string();

        // Compile syntax
        let syntax = obj.get("syntax").unwrap().as_str().unwrap();
        let syntax = CssSyntax::new(syntax)
            .compile()
            .unwrap_or_else(|_| panic!("Could not compile syntax for {name}: {syntax:?}"));

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
                resolved: false,
                shorthands: None,
            },
        );
    }

    properties
}

#[cfg(test)]
mod tests {
    use gosub_css3::colors::RgbColor;

    use super::*;

    macro_rules! assert_false {
        ($e:expr) => {
            assert_eq!(false, $e);
        };
    }

    macro_rules! assert_true {
        ($e:expr) => {
            assert_eq!(true, $e);
        };
    }

    macro_rules! str {
        ($s:expr) => {
            CssValue::String($s.to_string())
        };
    }

    macro_rules! unit {
        ($v:expr, $u:expr) => {
            CssValue::Unit($v, $u.to_string())
        };
    }

    #[test]
    fn test_parse_definition_file() {
        assert_eq!(CSS_DEFINITIONS.len(), 620);
    }

    #[test]
    fn test_prop_border() {
        let definitions = get_css_definitions();
        let prop = definitions.find_property("border").unwrap();

        assert!(prop.clone().matches(&[
            unit!(1.0, "px"),
            str!("solid"),
            CssValue::Color(RgbColor::from("black")),
        ]));
        assert!(prop.clone().matches(&[
            CssValue::Color(RgbColor::from("black")),
            str!("solid"),
            unit!(1.0, "px"),
        ]));
        assert!(prop.clone().matches(&[
            str!("solid"),
            CssValue::Color(RgbColor::from("black")),
            unit!(1.0, "px"),
        ]));
        assert!(prop.clone().matches(&[unit!(1.0, "px")]));
        assert!(prop.clone().matches(&[str!("solid")]));
        assert!(prop
            .clone()
            .matches(&[str!("solid"), CssValue::Color(RgbColor::from("black")),]));
        assert!(prop
            .clone()
            .matches(&[str!("solid"), CssValue::Color(RgbColor::from("black")),]));
        assert_true!(prop.clone().matches(&[str!("solid")]));
        assert_false!(prop.clone().matches(&[str!("not-solid")]));
        assert_false!(prop
            .clone()
            .matches(&[str!("solid"), str!("solid"), unit!(1.0, "px"),]));
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
                resolved: false,
                shorthands: None,
            },
        );
        definitions.resolve();

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
                initial_value: Some(str!("thick".to_string())),
                resolved: false,
                shorthands: None,
            },
        );

        definitions.resolve();

        assert_eq!(definitions.len(), 2);
        assert!(definitions.find_property("border-style").is_some());
    }

    /*
    #[test]
    fn test_azimuth() {
        let definitions = parse_definition_files();
        let def = definitions.find_property("azimuth").unwrap();

        assert_true!(def.clone().matches(vec![unit!(361.0, "deg")]));

        assert_false!(def.clone().matches(vec![unit!(20.0, "blaat")]));

        assert_true!(def
            .clone()
            .matches(vec![unit!(std::f32::consts::FRAC_PI_2, "rad")]));
        assert_true!(def.clone().matches(vec![CssValue::Zero]));

        assert_true!(def.clone().matches(vec![unit!(360.0, "deg")]));
        assert_true!(def.clone().matches(vec![unit!(360.0, "grad")]));
        assert_true!(def.clone().matches(vec![unit!(2.0, "grad")]));
        assert_true!(def.clone().matches(vec![unit!(-360.0, "grad")]));

        assert_false!(def.clone().matches(vec![str!("leftside")]));

        assert_true!(def.clone().matches(vec![str!("left-side")]));
        assert_true!(def.clone().matches(vec![str!("left")]));
        assert_true!(def.clone().matches(vec![str!("center")]));
        assert_true!(def.clone().matches(vec![str!("rightwards")]));
        assert_true!(def
            .clone()
            .matches(vec![str!("far-right"), str!("behind"),]));
        assert_true!(def.clone().matches(vec![str!("behind")]));
    }
     */

    #[test]
    fn test_background_color() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("background-color").unwrap();

        // assert_some!(def.clone().matches(&CssValue::Inherit));
        assert_true!(def.clone().matches(&[str!("transparent")]));

        assert_true!(def.clone().matches(&[str!("red")]));
        // System colors
        assert_true!(def.clone().matches(&[str!("Canvas")]));
        assert_true!(def.clone().matches(&[str!("CanvasText")]));
        assert_true!(def.clone().matches(&[str!("CanvasText")]));
        assert_true!(def.clone().matches(&[str!("Menu")]));

        assert_true!(def.clone().matches(&[str!("blue")]));
        assert_true!(def
            .clone()
            .matches(&[CssValue::Color(RgbColor::from("#ff0000"))]));
        assert_true!(def.clone().matches(&[str!("rebeccapurple")]));

        assert_false!(def.clone().matches(&[str!("thiscolordoesnotexist")]));
    }

    #[test]
    fn test_background_attachments() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("background-attachment").unwrap();

        // assert_true!(def.clone().matches(&CssValue::Inherit));
        assert_true!(def.clone().matches(&[str!("scroll")]));
        assert_true!(def.clone().matches(&[str!("fixed")]));

        assert_false!(def.clone().matches(&[str!("incorrect")]));
        assert_false!(def.clone().matches(&[str!("rebeccapurple")]));
        assert_false!(def.clone().matches(&[CssValue::Zero]));
    }

    #[test]
    fn test_background_position() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("background-position").unwrap();

        // background-position: left 10px;
        assert_true!(def.clone().matches(&[str!("left"), unit!(10.0, "px"),]));

        // background-position: left 10px top 20px;
        assert_true!(def.clone().matches(&[
            str!("left"),
            unit!(10.0, "px"),
            str!("top"),
            unit!(20.0, "px"),
        ]));

        // background-position: right 15% bottom 5%;
        assert_true!(def.clone().matches(&[
            str!("right"),
            CssValue::Percentage(15.0),
            str!("bottom"),
            CssValue::Percentage(5.0),
        ]));

        // background-position: center center;
        assert_true!(def.clone().matches(&[str!("center"), str!("center"),]));

        // background-position: 75% 50%;
        assert_true!(def
            .clone()
            .matches(&[CssValue::Percentage(75.0), CssValue::Percentage(50.0),]));

        // background-position: 75%;
        assert_true!(def.clone().matches(&[CssValue::Percentage(75.0),]));

        // background-position: top 10px center;
        assert_true!(def
            .clone()
            .matches(&[str!("top"), unit!(10.0, "px"), str!("center"),]));

        // background-position: bottom 20px right 30px;
        assert_true!(def.clone().matches(&[
            str!("bottom"),
            unit!(20.0, "px"),
            str!("right"),
            unit!(30.0, "px"),
        ]));

        // background-position: 20% 80%;
        assert_true!(def
            .clone()
            .matches(&[CssValue::Percentage(20.0), CssValue::Percentage(80.0),]));

        // background-position: left 5px bottom 15px, right 10px top 20px;
        assert_true!(def.clone().matches(&[
            str!("left"),
            unit!(5.0, "px"),
            str!("bottom"),
            unit!(15.0, "px"),
            CssValue::Comma,
            str!("right"),
            unit!(10.0, "px"),
            str!("top"),
            unit!(20.0, "px"),
        ]));

        // background-position: center top 35px;
        assert_true!(def
            .clone()
            .matches(&[str!("center"), str!("top"), unit!(35.0, "px"),]));

        // background-position: left 45% bottom 25%;
        assert_true!(def.clone().matches(&[
            str!("left"),
            CssValue::Percentage(45.0),
            str!("bottom"),
            CssValue::Percentage(25.0),
        ]));

        // background-position: right 10% top 50px;
        assert_true!(def.clone().matches(&[
            str!("right"),
            CssValue::Percentage(10.0),
            str!("top"),
            unit!(50.0, "px"),
        ]));

        // // background-position: 0% 0%, 100% 100%;
        assert_true!(def.clone().matches(&[
            CssValue::Percentage(0.0),
            CssValue::Percentage(0.0),
            CssValue::Comma,
            CssValue::Percentage(100.0),
            CssValue::Percentage(100.0),
        ]));

        // background-position: left top, right bottom;
        assert_true!(def.clone().matches(&[
            str!("left"),
            str!("top"),
            CssValue::Comma,
            str!("right"),
            str!("bottom"),
        ]));

        // background-position: 100% 0, 0 100%;
        assert_true!(def.clone().matches(&[
            CssValue::Percentage(100.0),
            CssValue::Zero,
            CssValue::Comma,
            CssValue::Zero,
            CssValue::Percentage(100.0),
        ]));

        // background-position: left 25px bottom, center top;
        assert_true!(def.clone().matches(&[
            str!("left"),
            unit!(25.0, "px"),
            str!("bottom"),
            CssValue::Comma,
            str!("center"),
            str!("top"),
        ]));

        // background-position: top 10% left 20%, bottom 10% right 20%;
        assert_true!(def.clone().matches(&[
            str!("top"),
            CssValue::Percentage(10.0),
            str!("left"),
            CssValue::Percentage(20.0),
            CssValue::Comma,
            str!("bottom"),
            CssValue::Percentage(10.0),
            str!("right"),
            CssValue::Percentage(20.0),
        ]));

        // background-position: 10px 30px, 90% 10%;
        assert_true!(def.clone().matches(&[
            unit!(10.0, "px"),
            unit!(30.0, "px"),
            CssValue::Comma,
            CssValue::Percentage(90.0),
            CssValue::Percentage(10.0),
        ]));

        // background-position: top right, bottom left 15px;
        assert_true!(def.clone().matches(&[
            str!("top"),
            str!("right"),
            CssValue::Comma,
            str!("bottom"),
            str!("left"),
            unit!(15.0, "px"),
        ]));

        // background-position: 50% 25%, 25% 75%;
        assert_true!(def.clone().matches(&[
            CssValue::Percentage(50.0),
            CssValue::Percentage(25.0),
            CssValue::Comma,
            CssValue::Percentage(25.0),
            CssValue::Percentage(75.0),
        ]));

        // background-position: right 5% bottom 5%, left 5% top 5%;
        assert_true!(def.clone().matches(&[
            str!("right"),
            CssValue::Percentage(5.0),
            str!("bottom"),
            CssValue::Percentage(5.0),
            CssValue::Comma,
            str!("left"),
            CssValue::Percentage(5.0),
            str!("top"),
            CssValue::Percentage(5.0),
        ]));
    }

    #[test]
    fn test_margin() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("margin").unwrap();

        println!("margin def: {:?}", def.syntax);

        assert!(def.clone().matches(&[unit!(1.0, "px")]));

        assert!(def.clone().matches(&[unit!(1.0, "px"), unit!(2.0, "px")]));

        assert!(def
            .clone()
            .matches(&[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px")]));

        assert!(def.clone().matches(&[
            unit!(1.0, "px"),
            unit!(2.0, "px"),
            unit!(3.0, "px"),
            unit!(4.0, "px"),
        ]));
    }
}
