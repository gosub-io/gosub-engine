use std::collections::HashMap;
use std::hash::Hash;
use std::sync::LazyLock;

use log::warn;

use crate::matcher::shorthands::{FixList, Shorthands};
use crate::matcher::syntax::GroupCombinators::Juxtaposition;
use crate::matcher::syntax::{CssSyntax, SyntaxComponent};
use crate::matcher::syntax_matcher::CssSyntaxTree;
use crate::stylesheet::CssValue;

/// Terminal data types that have no expandable grammar and are matched directly by
/// the syntax matcher rather than resolved from a value definition. This holds only:
/// genuine token primitives (`length`, `number`, `angle`, …), types with dedicated
/// match logic (`named-color`, `system-color`, `color()`, `hex-color`, `alpha()`),
/// and a few legacy/niche types no data source defines. Every value type that *does*
/// have a grammar is resolved from the definitions (webref / MDN `syntaxes.json`), so
/// it must NOT be listed here.
const BUILTIN_DATA_TYPES: [&str; 41] = [
    "age",
    "angle",
    "custom-ident",
    "dashed-ident",
    "decibel",
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
    "percentage",
    "string",
    "target-name",
    "time",
    "uri",
    "url-set",
    "url-token",
    "x",
    "y",
    "declaration-value",
    "number-token",
    "autospace",
    "dimension",
    "resolution",
    "url",
    // An alpha value is a numeric leaf (`<number> | <percentage>`) with no
    // expandable grammar; matched explicitly in syntax_matcher so it can't act as
    // a wildcard inside `<color-function>`.
    "alpha()",
    // Leaf datatypes that only appear inside function-argument grammars (attr(),
    // element(), calc-size(), dynamic-range-limit-mix(), @property `<syntax>`). No
    // data source (webref/MDN) defines them, so they are matched opaquely.
    "attr-name",
    "attr-unit",
    "dynamic-range-limit",
    "hash-token",
    "intrinsic-size-keyword",
    "syntax",
    "zero",
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
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn expanded_properties(&self) -> Vec<String> {
        self.computed.clone()
    }

    #[must_use]
    pub fn syntax(&self) -> &CssSyntaxTree {
        &self.syntax
    }

    #[must_use]
    pub fn inherited(&self) -> bool {
        self.inherited
    }

    /// Returns true when this definition has an initial value
    #[must_use]
    pub fn has_initial_value(&self) -> bool {
        self.initial_value.is_some()
    }

    /// Returns the initial value
    #[must_use]
    pub fn initial_value(&self) -> CssValue {
        self.initial_value.clone().unwrap_or(CssValue::None)
    }

    /// Matches a list of values against the current definition
    #[must_use]
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

    #[must_use]
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
    pub resolved: bool,
    pub ty: SyntaxType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxType {
    Quoted,
    Definition,
    None,
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
    /// Datatypes currently being resolved, used to break reference cycles in
    /// self-referential grammars (e.g. the calc() family). Transient during `resolve`.
    resolving: std::collections::HashSet<String>,
}

impl Default for CssDefinitions {
    fn default() -> Self {
        Self::new()
    }
}

impl CssDefinitions {
    #[must_use]
    pub fn new() -> Self {
        CssDefinitions {
            resolved_properties: HashMap::new(),
            properties: HashMap::new(),
            syntax: HashMap::new(),
            resolving: std::collections::HashSet::new(),
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
    #[must_use]
    pub fn find_property(&self, name: &str) -> Option<&PropertyDefinition> {
        self.resolved_properties.get(name)
    }

    /// Returns the length of the property definitions
    #[must_use]
    pub fn len(&self) -> usize {
        self.resolved_properties.len()
    }

    /// Returns true when the properties definitions are empty
    #[must_use]
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

        // Resolve the element
        let Some(mut element) = self.properties.get(name).cloned() else {
            // Property not found to resolve
            return;
        };
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
    pub fn resolve_component(&mut self, component: &SyntaxComponent, prop_name: &str) -> SyntaxComponent {
        match component {
            SyntaxComponent::Definition {
                datatype, multipliers, ..
            } => {
                // First step: Resolve by looking the definition up in the syntax defintions.
                if let Some(syntax_element) = self.syntax.get(datatype) {
                    // Cycle guard: if this datatype is already being resolved further up
                    // the stack, don't recurse into it again. Self-referential grammars
                    // (notably the calc() family, now reachable because function arguments
                    // are resolved) would otherwise recurse forever. Leaving the reference
                    // as an unresolved Definition is harmless: it only occurs at the
                    // recursive position of such grammars (e.g. calc(), whose arguments are
                    // kept as an opaque string and never matched against this grammar).
                    if self.resolving.contains(datatype) {
                        return component.clone();
                    }

                    let mut syntax_element = syntax_element.clone();
                    if !syntax_element.resolved {
                        self.resolving.insert(datatype.clone());
                        syntax_element.syntax = self.resolve_syntax(&syntax_element.syntax, prop_name);
                        syntax_element.resolved = true;
                        self.resolving.remove(datatype);
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

                #[allow(clippy::panic)]
                // PANIC-SAFE: datatypes come from the compiled-in definitions; the test suite resolves them all
                {
                    panic!("Unknown datatype encountered: {datatype:?}");
                }
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
                    combinator: *combinator,
                    multipliers: multipliers.clone(),
                }
            }
            SyntaxComponent::Function {
                name,
                arguments,
                multipliers,
            } => {
                // Resolve the argument grammar too, otherwise it keeps unresolved
                // `<datatype>` definitions that the matcher cannot match against.
                SyntaxComponent::Function {
                    name: name.clone(),
                    arguments: arguments
                        .as_ref()
                        .map(|arg| Box::new(self.resolve_component(arg, prop_name))),
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

pub static CSS_DEFINITIONS: LazyLock<CssDefinitions, fn() -> CssDefinitions> = LazyLock::new(parse_definition_files);

#[cfg(feature = "unresolved_syntax")]
pub static CSS_VALUES: LazyLock<indexmap::IndexMap<String, SyntaxDefinition>> = LazyLock::new(get_values);
#[cfg(feature = "unresolved_syntax")]
pub static CSS_PROPERTIES: LazyLock<indexmap::IndexMap<String, PropertyDefinition>> = LazyLock::new(get_properties);

pub const DEFINITIONS_VALUES: &str = include_str!("../../resources/definitions/definitions_values.json");
pub const DEFINITIONS_PROPERTIES: &str = include_str!("../../resources/definitions/definitions_properties.json");

#[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in definitions file, validated by the test suite
fn get_values<M: Map<String, SyntaxDefinition>>() -> M {
    let json: serde_json::Value = serde_json::from_str(DEFINITIONS_VALUES).expect("JSON was not well-formatted");
    parse_syntax_file(json)
}

#[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in definitions file, validated by the test suite
fn get_properties<M: Map<String, PropertyDefinition>>() -> M {
    let json: serde_json::Value = serde_json::from_str(DEFINITIONS_PROPERTIES).expect("JSON was not well-formatted");
    parse_property_file(json)
}

/// Parses the internal CSS definition file
fn parse_definition_files() -> CssDefinitions {
    // parse all syntax, so we can use them in the properties
    let syntax = get_values();

    // Parse property definitions
    let properties = get_properties();

    // Create definition structure, and resolve all definitions
    let mut definitions = CssDefinitions {
        resolved_properties: HashMap::new(),
        properties,
        syntax,
        resolving: std::collections::HashSet::new(),
    };

    definitions.index_shorthands();
    definitions.resolve();

    definitions
}

/// Main function to return the definitions. This will automatically load the definition files
/// and caches them if needed.
#[must_use]
pub fn get_css_definitions() -> &'static CssDefinitions {
    &CSS_DEFINITIONS
}

#[cfg(feature = "unresolved_syntax")]
pub fn get_css_values() -> &'static indexmap::IndexMap<String, SyntaxDefinition> {
    &CSS_VALUES
}

#[cfg(feature = "unresolved_syntax")]
pub fn get_css_properties() -> &'static indexmap::IndexMap<String, PropertyDefinition> {
    &CSS_PROPERTIES
}

trait Map<K, V> {
    fn new() -> Self;

    fn insert(&mut self, key: K, value: V);
}

impl<K: Eq + Hash, V> Map<K, V> for HashMap<K, V> {
    fn new() -> Self {
        HashMap::new()
    }

    fn insert(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

#[cfg(feature = "unresolved_syntax")]
impl<K: Eq + Hash, V> Map<K, V> for indexmap::IndexMap<K, V> {
    fn new() -> Self {
        indexmap::IndexMap::new()
    }

    fn insert(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
}

/// Parses a syntax JSON import file
#[allow(clippy::unwrap_used)] // PANIC-SAFE: parses the compiled-in definitions; validated by the test suite
fn parse_syntax_file<M: Map<String, SyntaxDefinition>>(json: serde_json::Value) -> M {
    let mut syntaxes = M::new();

    let entries = json.as_array().unwrap();
    for entry in entries {
        let syntax_str = entry.get("syntax").unwrap().as_str().unwrap();
        if syntax_str.is_empty() {
            continue;
        }
        match CssSyntax::new(syntax_str).compile() {
            Ok(ast) => {
                let mut name = entry.get("name").unwrap().to_string();
                let mut ty = SyntaxType::None;

                if name.starts_with('"') {
                    name = name[1..].to_string();
                    ty = SyntaxType::Quoted;
                }

                if name.starts_with('<') {
                    name = name[1..].to_string();
                    ty = SyntaxType::Definition;
                }

                if name.ends_with('"') {
                    name.pop();
                }

                if name.ends_with('>') {
                    name.pop();
                }

                // Genuine token primitives are matched directly by the syntax matcher.
                // Don't let a value definition of the same name shadow the builtin: MDN,
                // for instance, defines `integer` as `<number-token>`, which would make
                // `<integer>` accept any token and defeat validation.
                if BUILTIN_DATA_TYPES.contains(&name.as_str()) {
                    continue;
                }

                syntaxes.insert(
                    name.clone(),
                    SyntaxDefinition {
                        // name,
                        syntax: ast.clone(),
                        resolved: false,
                        ty,
                    },
                );
            }
            Err(e) => {
                // Type-definition compilation failures are expected for some advanced CSS
                // grammar constructs (e.g. structural `{ }` blocks in @keyframes, bare `)`
                // literals inside `[ ]` in <general-enclosed>). These types are not used
                // in property value matching anyway, so log at debug rather than warn.
                log::debug!(
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
#[allow(clippy::unwrap_used, clippy::panic)] // PANIC-SAFE: parses the compiled-in definitions; validated by the test suite
fn parse_property_file<M: Map<String, PropertyDefinition>>(json: serde_json::Value) -> M {
    let mut properties = M::new();

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
            warn!("Computed property is not a string or array {obj:?}");
            vec![]
        };

        let initial_value = if obj["initial_value"].is_array() {
            warn!("Initial value is an array, not supported {obj:?}");
            None
        } else if obj["initial_value"].is_string() {
            match CssValue::parse_str(obj["initial_value"].as_str().unwrap()) {
                Ok(value) => Some(value),
                Err(e) => {
                    warn!("Could not parse initial value: {e:?}");
                    None
                }
            }
        } else {
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
    use super::*;
    use crate::colors::RgbColor;

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
        // set the log level to WARN
        simple_logger::SimpleLogger::new()
            .with_level(log::LevelFilter::Warn)
            .init()
            .unwrap();
        assert_eq!(CSS_DEFINITIONS.len(), 666);
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
        assert_false!(prop.clone().matches(&[str!("solid"), str!("solid"), unit!(1.0, "px"),]));
    }

    /// Parse a real declaration `prop: value` and return its value list, exactly as
    /// the cascade would see it (functions like `rgb(0,0,0)` collapse to `Color`).
    fn parse_decl_values(prop: &str, value: &str) -> Vec<CssValue> {
        use crate::Css3;
        use gosub_interface::css3::CssOrigin;
        use gosub_shared::config::ParserConfig;

        let css = format!("x {{ {prop}: {value}; }}");
        let config = ParserConfig {
            match_values: false,
            ignore_errors: true,
            ..Default::default()
        };
        let sheet = Css3::parse_str(&css, config, CssOrigin::Author, "corpus-test").expect("parse");
        let Some(decl) = sheet.rules.first().and_then(|r| r.declarations.first()) else {
            return vec![];
        };
        match &decl.value {
            CssValue::List(v) => v.clone(),
            other => vec![other.clone()],
        }
    }

    /// A broad corpus exercising the matcher across many property/value combinations,
    /// including the newly data-driven value types. Prints every mismatch, then fails
    /// if any case disagrees with its expectation.
    /// Run with: cargo test -p gosub_css3 test_matcher_corpus --lib -- --nocapture
    #[test]
    fn test_matcher_corpus() {
        // property -> list of (value, should-match)
        let corpus: &[(&str, &[(&str, bool)])] = &[
            (
                "width",
                &[
                    ("10px", true),
                    ("0", true),
                    ("5em", true),
                    ("10.42%", true),
                    ("auto", true),
                    ("min-content", true),
                    ("fit-content(10px)", true),
                    ("fit-content(50%)", true),
                    // argument is validated: a color is not a <length-percentage>
                    ("fit-content(red)", false),
                    ("banana", false),
                    ("rgb(0,0,0)", false),
                    ("red", false),
                ],
            ),
            (
                "color",
                &[
                    ("red", true),
                    ("rebeccapurple", true),
                    ("#ff0000", true),
                    ("rgb(0,0,0)", true),
                    ("rgba(0,0,0,0.5)", true),
                    ("hsl(0,0%,0%)", true),
                    ("transparent", true),
                    ("banana", false),
                    ("10px", false),
                ],
            ),
            (
                "display",
                &[
                    ("block", true),
                    ("inline-block", true),
                    ("flex", true),
                    ("grid", true),
                    ("none", true),
                    ("banana", false),
                    ("10px", false),
                ],
            ),
            (
                "font-size",
                &[
                    ("12px", true),
                    ("1.5em", true),
                    ("large", true),
                    ("120%", true),
                    ("banana", false),
                ],
            ),
            (
                "opacity",
                &[("0.5", true), ("1", true), ("0", true), ("banana", false)],
            ),
            (
                "z-index",
                &[("10", true), ("auto", true), ("banana", false), ("1.5", false)],
            ),
            (
                "position",
                &[("absolute", true), ("relative", true), ("static", true), ("banana", false)],
            ),
            (
                "text-align",
                &[("center", true), ("left", true), ("justify", true), ("banana", false)],
            ),
            (
                "overflow",
                &[("hidden", true), ("scroll", true), ("auto", true), ("banana", false)],
            ),
            (
                "line-height",
                &[("1.5", true), ("normal", true), ("20px", true), ("150%", true)],
            ),
            (
                "aspect-ratio",
                &[("auto", true), ("16 / 9", true), ("1", true), ("banana", false)],
            ),
            (
                "rotate",
                &[("45deg", true), ("none", true), ("1turn", true), ("banana", false)],
            ),
            (
                "transition-timing-function",
                &[
                    ("ease", true),
                    ("linear", true),
                    // multi-argument functions: comma-separated arguments must match
                    ("cubic-bezier(0.1, 0.7, 1, 0.1)", true),
                    ("steps(4, end)", true),
                    ("banana", false),
                ],
            ),
            (
                "box-shadow",
                &[
                    ("2px 2px", true),
                    ("2px 2px 4px red", true),
                    ("inset 2px 2px 4px red", true),
                    ("2px 2px red, 3px 3px blue", true),
                    ("banana", false),
                ],
            ),
            (
                // `<'margin-top'>{1,4}` — the 1-to-4 value box shorthand (ranged multiplier).
                "margin",
                &[
                    ("10px", true),
                    ("10px 20px", true),
                    ("1px 2px 3px", true),
                    ("1px 2px 3px 4px", true),
                    ("auto", true),
                    ("0 auto", true),
                    // A fifth value exceeds the {1,4} range.
                    ("1px 2px 3px 4px 5px", false),
                    ("banana", false),
                ],
            ),
            (
                // Same {1,4} shorthand shape, length-percentage only (no `auto`).
                "padding",
                &[("10px", true), ("1px 2px 3px 4px", true), ("auto", false)],
            ),
            (
                // `none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]` — a `||`
                // any-order group with an embedded optional operand.
                "flex",
                &[
                    ("1", true),
                    ("1 1", true),
                    ("1 1 0%", true),
                    ("auto", true),
                    ("none", true),
                    ("banana", false),
                ],
            ),
            (
                // `<single-transition>#`. A single transition uses a `||` group of
                // property/time/easing. ("banana" is a valid <custom-ident>
                // transition-property, so it legitimately matches.)
                "transition",
                &[
                    ("all 0.3s ease", true),
                    ("opacity 0.3s", true),
                    ("0.3s", true),
                    ("banana", true),
                ],
            ),
            (
                // `[ ... <'font-size'> [ / <'line-height'> ]? <'font-family'># ] | ...`
                // — exercises the `/` line-height separator inside a shorthand.
                "font",
                &[("12px serif", true), ("italic bold 12px/1.5 serif", true), ("banana", false)],
            ),
        ];

        let defs = get_css_definitions();
        let (mut total, mut mismatches) = (0usize, Vec::new());
        for (prop, cases) in corpus {
            let Some(def) = defs.find_property(prop) else {
                mismatches.push(format!("NO-DEF for {prop}"));
                continue;
            };
            for (value, expected) in *cases {
                total += 1;
                let values = parse_decl_values(prop, value);
                let got = def.clone().matches(&values);
                if got != *expected {
                    mismatches.push(format!("{prop}: {value:?} -> match={got} (want {expected})"));
                }
            }
        }

        eprintln!("test_matcher_corpus: {}/{} passed", total - mismatches.len(), total);
        for m in &mismatches {
            eprintln!("  MISMATCH {m}");
        }
        assert!(mismatches.is_empty(), "{} matcher mismatches", mismatches.len());
    }

    /// Coverage for the multipliers not exercised by the combinator regression test
    /// (`*` zero-or-more, `!` at-least-one-in-group, and the ranged `{min,max}` form —
    /// as opposed to the fixed `{2}` count tested elsewhere).
    #[test]
    fn test_multiplier_coverage() {
        fn m(grammar: &str, vals: &[CssValue]) -> bool {
            CssSyntax::new(grammar).compile().expect("compile").matches(vals)
        }
        let (a, b) = (|| str!("a"), || str!("b"));

        // `*` — zero or more, so the empty input is valid.
        assert!(m("a*", &[]));
        assert!(m("a*", &[a()]));
        assert!(m("a*", &[a(), a(), a()]));

        // `!` — the group must produce at least one value even though every operand
        // inside it is individually optional.
        assert!(!m("[ a? b? ]!", &[]));
        assert!(m("[ a? b? ]!", &[a()]));
        assert!(m("[ a? b? ]!", &[b()]));
        assert!(m("[ a? b? ]!", &[a(), b()]));

        // `{min,max}` — a genuine range (not the fixed `{2}` count): 1..=3 here.
        assert!(!m("a{1,3}", &[]));
        assert!(m("a{1,3}", &[a()]));
        assert!(m("a{1,3}", &[a(), a(), a()]));
        assert!(!m("a{1,3}", &[a(), a(), a(), a()]));
    }

    /// Documents the matcher's currently-accepted limitations so the behavior is
    /// pinned and the divergence from spec-correct matching is discoverable. Each
    /// assertion encodes CURRENT behavior; flipping one when the underlying gap is
    /// fixed is the intended maintenance signal.
    #[test]
    fn test_matcher_known_limitations() {
        let defs = get_css_definitions();
        let ok = |prop: &str, v: &str| {
            defs.find_property(prop)
                .unwrap_or_else(|| panic!("no def for {prop}"))
                .clone()
                .matches(&parse_decl_values(prop, v))
        };

        // Numeric range constraints (`<length [0,∞]>`) are NOT enforced: the matcher
        // treats the bounded datatype as its unbounded base, so a negative length for a
        // non-negative property still matches.
        assert!(ok("width", "-5px"));

        // A comma-separated list of a value type that itself contains a `||` group does
        // not split on the separating comma (same family as the box-shadow `#` gap):
        // a single transition matches, but a multi-transition list does not.
        assert!(ok("transition", "opacity 0.3s"));
        assert!(!ok("transition", "opacity 0.3s, transform 0.5s"));

        // The `fr` flex unit (`<flex>`) is not matched, so a track list using it fails
        // even though a length/keyword track list matches.
        assert!(ok("grid-template-columns", "100px auto"));
        assert!(!ok("grid-template-columns", "1fr 1fr"));

        // webref types `background` as `<bg-layer>#? , <final-bg-layer>`, which makes the
        // separating comma mandatory, so a bare single-layer background does not match.
        assert!(!ok("background", "red"));
    }

    /// Regression tests for combinator/multiplier matching bugs fixed alongside
    /// box-shadow support.
    #[test]
    fn test_combinator_and_multiplier_matching() {
        fn m(grammar: &str, vals: &[CssValue]) -> bool {
            CssSyntax::new(grammar).compile().expect("compile").matches(vals)
        }
        // `&&` with an absent trailing optional operand.
        assert!(m("a && b?", &[str!("a")]));
        // `&&` where optional operands surround a multi-value operand.
        assert!(m("a? && [ b{2} ] && c?", &[str!("b"), str!("b")]));
        // A single-element group must keep its inner multiplier (`[ b{2} ]` == `b{2}`).
        assert!(m("[ b{2} ]", &[str!("b"), str!("b")]));
        assert!(!m("[ b{2} ]", &[str!("b")]));
        // `#` list stops at a non-comma and yields the rest to the next component.
        assert!(m("b# c?", &[str!("b"), str!("c")]));
        assert!(m("b# c", &[str!("b"), str!("c")]));
        // A bounded multiplier must not greedily consume beyond its maximum.
        assert!(m("a{2} b", &[str!("a"), str!("a"), str!("b")]));
        // `&&` with an optional operand whose real value appears after other operands.
        assert!(m("a? && b{2}", &[str!("b"), str!("b"), str!("a")]));
    }

    #[test]
    fn test_box_shadow_matching() {
        let defs = get_css_definitions();
        let def = defs.find_property("box-shadow").unwrap();
        let ok = |v: &str| def.clone().matches(&parse_decl_values("box-shadow", v));

        // Single shadow, all component combinations (offset / blur / spread / color /
        // position, in any order).
        assert!(ok("2px 2px"));
        assert!(ok("2px 2px 4px"));
        assert!(ok("2px 2px 4px 5px"));
        assert!(ok("2px 2px 4px red"));
        assert!(ok("red 2px 2px"));
        assert!(ok("inset 2px 2px"));
        assert!(ok("inset 2px 2px 4px red"));
        // Comma-separated list of shadows, with and without per-shadow colors.
        // webref decomposes box-shadow into sub-properties typed as comma-lists
        // (`box-shadow-color = <color>#`); when embedded in one shadow, that inner `#`
        // used to greedily consume the shadow-separating comma. The generator now strips
        // the trailing `#` from those value-type definitions, so both cases match.
        assert!(ok("2px 2px, 3px 3px"));
        assert!(ok("2px 2px red, 3px 3px blue"));
        // Invalid.
        assert!(!ok("banana"));
    }

    #[test]
    fn test_property_definitions() {
        let mut definitions = CssDefinitions::new();
        definitions.add_property(
            "color",
            PropertyDefinition {
                name: "color".to_string(),
                computed: vec![],
                syntax: CssSyntax::new("color()").compile().expect("Could not compile syntax"),
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
                syntax: CssSyntax::new("").compile().expect("Could not compile syntax"),
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

        assert_true!(def.clone().matches(&[str!("transparent")]));

        assert_true!(def.clone().matches(&[str!("red")]));
        // System colors
        assert_true!(def.clone().matches(&[str!("Canvas")]));
        assert_true!(def.clone().matches(&[str!("CanvasText")]));
        assert_true!(def.clone().matches(&[str!("CanvasText")]));
        assert_true!(def.clone().matches(&[str!("Menu")]));

        assert_true!(def.clone().matches(&[str!("blue")]));
        assert_true!(def.clone().matches(&[CssValue::Color(RgbColor::from("#ff0000"))]));
        assert_true!(def.clone().matches(&[str!("rebeccapurple")]));

        assert_false!(def.clone().matches(&[str!("thiscolordoesnotexist")]));
    }

    #[test]
    fn test_background_attachments() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("background-attachment").unwrap();

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
        assert_true!(def
            .clone()
            .matches(&[str!("left"), unit!(10.0, "px"), str!("top"), unit!(20.0, "px"),]));

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
        assert_true!(def.clone().matches(&[str!("top"), unit!(10.0, "px"), str!("center"),]));

        // background-position: bottom 20px right 30px;
        assert_true!(def
            .clone()
            .matches(&[str!("bottom"), unit!(20.0, "px"), str!("right"), unit!(30.0, "px"),]));

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
        assert_true!(def.clone().matches(&[str!("center"), str!("top"), unit!(35.0, "px"),]));

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

        assert!(def
            .clone()
            .matches(&[unit!(1.0, "px"), unit!(2.0, "px"), unit!(3.0, "px"), unit!(4.0, "px"),]));
    }

    #[test]
    fn test_font_var() {
        let definitions = get_css_definitions();
        let def = definitions.find_property("font-variation-settings").unwrap();

        assert_true!(def.matches(&[str!("normal")]));

        assert_true!(def.matches(&[str!("wgth"), CssValue::Number(100.0)]));

        assert_true!(def.matches(&[
            str!("wgth"),
            CssValue::Number(100.0),
            CssValue::Comma,
            str!("ital"),
            CssValue::Number(100.0)
        ]));
    }
}
