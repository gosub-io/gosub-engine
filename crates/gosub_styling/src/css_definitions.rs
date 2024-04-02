use std::collections::HashMap;
use gosub_css3::colors::RgbColor;
use gosub_css3::stylesheet::CssValue;
use gosub_shared::types::{Error, Result};

#[allow(dead_code)]
#[derive(Debug)]
struct PropertyDefinition {
    name: String,
    expanded_values: Vec<String>,
    type_: Vec<String>,
    type_parser: String,
    inherits: bool,
    initial_value: CssValue,
}

fn parse_definition_file() -> HashMap<String, PropertyDefinition> {
    println!("parse_definition_file");

    let contents = include_str!("css_definitions.json");
    let json: serde_json::Value = serde_json::from_str(&contents).expect("JSON was not well-formatted");

    let mut definitions = HashMap::new();

    let entries = json.as_array().unwrap();
    for entry in entries {
        let name = entry["name"].as_str().unwrap().to_string();
        println!("name: {}", name);

        let mut expanded_values = vec![];
        let mut type_: Vec<String> = vec![];
        let mut type_parser: String = "".to_string();
        let mut inherits: bool = false;
        let mut initial_value: CssValue = CssValue::None;

        if let Some(value) = entry["expanded_values"].as_array() {
            expanded_values = value.iter().map(|v| v.to_string()).collect();
        }
        if let Some(value) = entry["type"].as_str() {
            type_ = value
                .split('|')
                .filter(|&v| check_type_definition(v))
                .map(|v| v.to_string())
                .collect()
            ;
        }
        if let Some(value) = entry["convert_value"].as_str() {
            type_parser = value.to_string();
        }
        if let Some(value) = entry["inherits"].as_bool() {
            inherits = value;
        }
        if let Some(value) = entry["initial_value"].as_str() {
            initial_value = convert_value(value).unwrap();
        }

        definitions.insert(name.clone(), PropertyDefinition {
            name: name.clone(),
            expanded_values,
            type_,
            type_parser,
            inherits,
            initial_value,
        });
    }

    definitions
}

fn check_type_definition(definition: &str) -> bool {
    check_type(definition, None)
}

fn check_type_value(definition: &str, value: &CssValue) -> bool {
    check_type(definition, Some(value))
}

fn check_type(definition: &str, value: Option<&CssValue>) -> bool {
    // Literal strings should match exactly
    if definition.starts_with("'") && definition.ends_with("'") {
        if value.is_none() {
            return true;
        }

        if let Some(CssValue::String(s)) = value {
            return s == &definition[1..definition.len()-1];
        }

        return false;
    }

    // Any color is ok
    if definition == "color" {
        if value.is_none() {
            return true;
        }

        if let Some(CssValue::Color(_)) = value {
            return true;
        }
        return false;
    }

    // Floats
    if definition == "number" {
        if value.is_none() {
            return true;
        }

        if let Some(CssValue::Number(_)) = value {
            return true;
        }
        return false;
    }

    // Any number with optional range (e.g. number(42), number(0..100), number(..100), number(0..))
    if definition.starts_with("number(") && definition.ends_with(")") {
        if value.is_none() {
            return true;
        }

        if let Some(CssValue::Number(f)) = value {
            return is_in_range(&definition[7..definition.len() - 1], *f)
        }
        return false;
    }

    // Percentages
    if definition == "percentage" {
        if value.is_none() {
            return true;
        }

        if let Some(CssValue::Percentage(_)) = value {
            return true;
        }
        return false;
    }

    // Units
    if definition == "unit" {
        if value.is_none() {
            return true;
        }

        // Any unit will do
        if let Some(CssValue::Unit(_, _)) = value {
            return true;
        }
        return false;
    }

    // Any unit with optional range (e.g. number(42), number(0..100), number(..100), number(0..)) and unit
    if definition.starts_with("unit(") && definition.ends_with(")") {
        let mut parts = definition[5..definition.len()-1].splitn(2, ' ').collect::<Vec<&str>>();
        if parts.len() == 1 {
            parts.insert(0, "")
        }

        if value.is_none() {
            return true;
        }


        if let Some(CssValue::Unit(f, u)) = value {
            // the unit part can have multiple units (e.g. px|em|vh)
            let allowed_units = parts[1].split('|').collect::<Vec<&str>>();

            if parts[0] == "" {
                // No value
                return allowed_units.contains(&u.as_str())
            }

            // Check if the value is in range and the unit is allowed
            return is_in_range(&parts[0], *f) && allowed_units.contains(&u.as_str());
        }

        return false;
    }

    // None
    if definition == "none" && value == Some(&CssValue::None) {
        return true;
    }

    // Nothing found that matched
    false
}

fn convert_value(value: &str) -> Result<CssValue> {
    // Between single quotes is a literal string
    if value.starts_with("'") && value.ends_with("'") {
        return Ok(CssValue::String(value[1..value.len()-1].to_string()))
    }

    // Color values
    if value.starts_with("color(") && value.ends_with(")") {
        return Ok(CssValue::Color(RgbColor::from(value[6..value.len()-1].to_string().as_str())))
    }

    // Numbers (floats)
    let num = value.parse::<f32>();
    if num.is_ok() {
        return Ok(CssValue::Number(num.unwrap()))
    }

    // Percentages
    if value.ends_with('%') {
        let num = value[0..value.len()-1].parse::<f32>();
        if num.is_ok() {
            return Ok(CssValue::Percentage(num.unwrap()))
        }
    }

    // units
    if value.starts_with("unit(") && value.ends_with(")") {
        // split by space
        let parts: Vec<&str> = value[5..value.len()-1].splitn(2, ' ').collect();
        return Ok(CssValue::Unit(parts[0].parse().unwrap(), parts[1].to_string()))
    }

    // Explicit none value
    if value == "none" {
        return Ok(CssValue::None)
    }

    Err(Error::Parse(format!("Could not convert value: {}", value)).into())
}

/// Checks if the given value is in range
///
/// Examples:
///     100    -> exactly 100
///     0..100 -> 0 to 100
///     ..100  -> -MIN to 100
///     0..    -> 0 to MAX
fn is_in_range(range: &str, value: f32) -> bool {
    let parts = range.splitn(2, "..").collect::<Vec<&str>>();
    if parts.len() == 1 {
        return parts[0].parse::<f32>().unwrap() == value;
    }

    let min = if parts[0] == "" { f32::MIN } else { parts[0].parse().unwrap() };
    let max = if parts[1] == "" { f32::MAX } else { parts[1].parse().unwrap() };

    value >= min && value <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_in_range() {
        assert!(is_in_range("0..100", 42.0));
        assert!(is_in_range("0..100", 0.0));
        assert!(is_in_range("0..100", 100.0));
        assert!(!is_in_range("0..100", 101.0));
        assert!(!is_in_range("0..100", -1.0));

        assert!(is_in_range("0..", 42.0));
        assert!(is_in_range("0..", 0.0));
        assert!(!is_in_range("0..", -0.001));

        assert!(is_in_range("..100", 42.0));
        assert!(is_in_range("..100", 100.0));
        assert!(!is_in_range("..100", 100.1));
    }

    #[test]
    fn test_parse_definition_file() {
        let definitions = parse_definition_file();
        dbg!(&definitions);
        assert_eq!(definitions.len(), 18);
    }

    #[test]
    fn test_convert_value() {
        assert_eq!(convert_value("'hello'").unwrap(), CssValue::String("hello".to_string()));
        assert_eq!(convert_value("color(#ff0000)").unwrap(), CssValue::Color(RgbColor::from("#ff0000")));
        assert_eq!(convert_value("color(#ff0000)").unwrap(), CssValue::Color(RgbColor::from("red")));
        assert_eq!(convert_value("color(rebeccapurple)").unwrap(), CssValue::Color(RgbColor::from("#663399")));
        assert_eq!(convert_value("42").unwrap(), CssValue::Number(42.0));
        assert_eq!(convert_value("12.34").unwrap(), CssValue::Number(12.34));
        assert_eq!(convert_value("64.8%").unwrap(), CssValue::Percentage(64.8));
        assert_eq!(convert_value("unit(42 px)").unwrap(), CssValue::Unit(42.0, "px".to_string()));
        assert_eq!(convert_value("none").unwrap(), CssValue::None);

        assert!(convert_value("does-not-exists").is_err());
    }


    #[test]
    fn test_check_type_with_values() {
        assert!(check_type_value("'hello'", &CssValue::String("hello".to_string())));
        assert!(!check_type_value("'hello'", &CssValue::String("world".to_string())));

        assert!(check_type_value("color", &CssValue::Color(RgbColor::from("#ff0000"))));
        assert!(check_type_value("color", &CssValue::Color(RgbColor::from("red"))));
        assert!(!check_type_value("color", &CssValue::Number(42.0)));

        assert!(check_type_value("number", &CssValue::Number(42.0)));
        assert!(!check_type_value("number", &CssValue::String("hello".to_string())));
        assert!(!check_type_value("number", &CssValue::Color(RgbColor::from("red"))));

        assert!(check_type_value("number(0..100)", &CssValue::Number(42.0)));
        assert!(check_type_value("number(0..100)", &CssValue::Number(0.0)));
        assert!(check_type_value("number(0..100)", &CssValue::Number(100.0)));
        assert!(!check_type_value("number(0..100)", &CssValue::Number(101.0)));
        assert!(!check_type_value("number(0..100)", &CssValue::Number(-1.0)));

        assert!(check_type_value("number(0..)", &CssValue::Number(42.0)));
        assert!(check_type_value("number(0..)", &CssValue::Number(0.0)));
        assert!(!check_type_value("number(0..)", &CssValue::Number(-1.0)));

        assert!(check_type_value("number(..100)", &CssValue::Number(42.0)));
        assert!(check_type_value("number(..100)", &CssValue::Number(100.0)));
        assert!(!check_type_value("number(..100)", &CssValue::Number(101.0)));

        assert!(check_type_value("none", &CssValue::None));
        assert!(!check_type_value("none", &CssValue::Number(42.0)));

        assert!(check_type_value("percentage", &CssValue::Percentage(42.0)));
        assert!(!check_type_value("percentage", &CssValue::Number(42.0)));

        assert!(check_type_value("unit", &CssValue::Unit(525.0, "doesnotmatter".to_string())));
        assert!(check_type_value("unit(px)", &CssValue::Unit(42.0, "px".to_string())));
        assert!(check_type_value("unit(px)", &CssValue::Unit(-525.0, "px".to_string())));
        assert!(check_type_value("unit(px)", &CssValue::Unit(525.0, "px".to_string())));
        assert!(check_type_value("unit(px|em)", &CssValue::Unit(525.0, "em".to_string())));
        assert!(check_type_value("unit(px|em)", &CssValue::Unit(525.0, "px".to_string())));
        assert!(!check_type_value("unit(px|em)", &CssValue::Unit(525.0, "vh".to_string())));
        assert!(!check_type_value("unit(px)", &CssValue::Unit(42.0, "em".to_string())));

        assert!(check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(42.0, "px".to_string())));
        assert!(check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(0.0, "px".to_string())));
        assert!(check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(-1.0, "px".to_string())));
        assert!(check_type_value("unit(..100.2 px|em|vh)", &CssValue::Unit(100.1, "px".to_string())));
        assert!(check_type_value("unit(..100.2 px|em|vh)", &CssValue::Unit(100.2, "vh".to_string())));
        assert!(!check_type_value("unit(..100.2 px|em|vh)", &CssValue::Unit(100.21, "vh".to_string())));

        assert!(!check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(42.0, "foo".to_string())));
        assert!(!check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(0.0, "foo".to_string())));
        assert!(!check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(-1.0, "foo".to_string())));
        assert!(!check_type_value("unit(..100 px|em|vh)", &CssValue::Unit(100.1, "foo".to_string())));

        assert!(check_type_value("unit(100 px|em|vh)", &CssValue::Unit(100.0, "px".to_string())));
        assert!(check_type_value("unit(100 px|em|vh)", &CssValue::Unit(100.0, "em".to_string())));
        assert!(check_type_value("unit(100 px|em|vh)", &CssValue::Unit(100.0, "vh".to_string())));

        assert!(!check_type_value("unit(100 px|em|vh)", &CssValue::Unit(99.9, "px".to_string())));
        assert!(!check_type_value("unit(100 px|em|vh)", &CssValue::Unit(99.9, "em".to_string())));
        assert!(!check_type_value("unit(100 px|em|vh)", &CssValue::Unit(99.9, "vh".to_string())));
    }

    #[test]
    fn test_check_type_without_values() {
        assert!(check_type_definition("unit(100)"));
        assert!(check_type_definition("unit(em|px)"));
        assert!(check_type_definition("number"));
        assert!(check_type_definition("unit"));
        assert!(check_type_definition("percentage"));
    }
}