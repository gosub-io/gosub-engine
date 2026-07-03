use crate::errors::Error;
use core::fmt::Display;
use cow_utils::CowUtils;
use log::warn;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

/// A setting can be either a signed integer, unsigned integer, float, string, map or boolean.
/// Maps could be created by using comma separated strings maybe
#[derive(Clone, PartialEq, Debug)]
pub enum Setting {
    SInt(isize),
    UInt(usize),
    Float(f64),
    String(String),
    Bool(bool),
    Map(Vec<String>),
}

impl Setting {
    #[must_use]
    pub fn to_bool(&self) -> bool {
        if !matches!(self, Setting::Bool(_)) {
            warn!("setting is not a boolean");
        }

        match self {
            Setting::Bool(value) => *value,
            Setting::SInt(value) => *value != 0,
            Setting::UInt(value) => *value != 0,
            Setting::Float(value) => *value != 0.0,
            Setting::String(value) => is_bool_value(value),
            Setting::Map(values) => !values.is_empty(),
        }
    }

    #[must_use]
    pub fn to_sint(&self) -> isize {
        if !matches!(self, Setting::SInt(_)) {
            warn!("setting is not an signed integer");
        }

        match self {
            Setting::SInt(value) => *value,
            Setting::UInt(value) => *value as isize,
            Setting::Float(value) => *value as isize,
            Setting::Bool(value) => isize::from(*value),
            Setting::String(value) => isize::from(is_bool_value(value)),
            Setting::Map(values) => values.len() as isize,
        }
    }

    #[must_use]
    pub fn to_uint(&self) -> usize {
        if !matches!(self, Setting::UInt(_)) {
            warn!("setting is not an unsigned integer");
        }

        match self {
            Setting::UInt(value) => *value,
            Setting::SInt(value) => *value as usize,
            Setting::Float(value) => *value as usize,
            Setting::Bool(value) => usize::from(*value),
            Setting::String(value) => usize::from(is_bool_value(value)),
            Setting::Map(values) => values.len(),
        }
    }

    #[must_use]
    pub fn to_float(&self) -> f64 {
        if !matches!(self, Setting::Float(_)) {
            warn!("setting is not a float");
        }

        match self {
            Setting::Float(value) => *value,
            Setting::SInt(value) => *value as f64,
            Setting::UInt(value) => *value as f64,
            Setting::Bool(value) => f64::from(u8::from(*value)),
            Setting::String(value) => f64::from(u8::from(is_bool_value(value))),
            Setting::Map(values) => values.len() as f64,
        }
    }

    #[allow(clippy::inherent_to_string_shadow_display)]
    #[must_use]
    pub fn to_string(&self) -> String {
        if !matches!(self, Setting::String(_)) {
            warn!("setting is not a string");
        }

        match self {
            Setting::SInt(value) => value.to_string(),
            Setting::UInt(value) => value.to_string(),
            Setting::Float(value) => value.to_string(),
            Setting::String(value) => value.clone(),
            Setting::Bool(value) => value.to_string(),
            Setting::Map(values) => {
                let mut result = String::new();
                for value in values {
                    result.push_str(value);
                    result.push(',');
                }
                result.pop();
                result
            }
        }
    }

    #[must_use]
    pub fn to_map(&self) -> Vec<String> {
        if !matches!(self, Setting::Map(_)) {
            warn!("setting is not a map");
        }

        match self {
            Setting::Map(values) => values.clone(),
            other => vec![other.to_string()],
        }
    }

    /// Returns a human-readable name for this setting's type (e.g. `unsigned`, `float`).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Setting::SInt(_) => "signed",
            Setting::UInt(_) => "unsigned",
            Setting::Float(_) => "float",
            Setting::String(_) => "string",
            Setting::Bool(_) => "boolean",
            Setting::Map(_) => "map",
        }
    }

    /// Returns the bare value of this setting as a string, without the type prefix used by
    /// [`Display`] and without emitting a type-mismatch warning (unlike [`Setting::to_string`]).
    #[must_use]
    pub fn value_string(&self) -> String {
        match self {
            Setting::SInt(v) => v.to_string(),
            Setting::UInt(v) => v.to_string(),
            Setting::Float(v) => v.to_string(),
            Setting::String(v) => v.clone(),
            Setting::Bool(v) => v.to_string(),
            Setting::Map(v) => v.join(","),
        }
    }
}

fn is_bool_value(s: &str) -> bool {
    let us = s.cow_to_uppercase();
    if ["YES", "ON", "TRUE", "1"].contains(&us.as_ref()) {
        return true;
    }

    false
}

impl Serialize for Setting {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Setting::from_str(&value).map_err(|err| serde::de::Error::custom(format!("cannot deserialize: {err}")))
    }
}

impl Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setting::SInt(value) => write!(f, "i:{value}"),
            Setting::UInt(value) => write!(f, "u:{value}"),
            Setting::Float(value) => write!(f, "f:{value}"),
            Setting::String(value) => write!(f, "s:{value}"),
            Setting::Bool(value) => write!(f, "b:{value}"),
            Setting::Map(values) => write!(f, "m:{}", values.join(",")),
        }
    }
}

impl FromStr for Setting {
    type Err = Error;

    // first element is the type:
    //   b:true
    //   i:-123
    //   u:234
    //   f:1.5
    //   s:hello world
    //   m:foo,bar,baz

    /// Parses a prefixed string into a `Setting`. The format is `<type>:<value>`,
    /// e.g. `b:true`, `i:-1`, `u:42`, `f:1.5`, `s:hello`, `m:foo,bar`. Returns an error
    /// when the prefix is missing or the value cannot be parsed.
    fn from_str(key: &str) -> Result<Setting, crate::errors::Error> {
        let (key_type, key_value) = key
            .split_once(':')
            .ok_or_else(|| Error::Config(format!("invalid setting format, missing ':' in {key:?}")))?;

        let setting = match key_type {
            "b" => Setting::Bool(
                key_value
                    .parse::<bool>()
                    .map_err(|err| Error::Config(format!("error parsing {key_value}: {err}")))?,
            ),
            "i" => Setting::SInt(
                key_value
                    .parse::<isize>()
                    .map_err(|err| Error::Config(format!("error parsing {key_value}: {err}")))?,
            ),
            "u" => Setting::UInt(
                key_value
                    .parse::<usize>()
                    .map_err(|err| Error::Config(format!("error parsing {key_value}: {err}")))?,
            ),
            "f" => Setting::Float(
                key_value
                    .parse::<f64>()
                    .map_err(|err| Error::Config(format!("error parsing {key_value}: {err}")))?,
            ),
            "s" => Setting::String(key_value.to_string()),

            "m" => {
                if key_value.is_empty() {
                    Setting::Map(vec![])
                } else {
                    Setting::Map(key_value.split(',').map(str::to_string).collect())
                }
            }
            _ => return Err(Error::Config(format!("unknown setting: {key_value}"))),
        };

        Ok(setting)
    }
}

/// Restricts the values a setting may be set to. Built from the optional `values` field in
/// `settings.json`.
#[derive(Clone, PartialEq, Debug)]
pub enum Constraint {
    /// The setting's string form must equal one of these literals (e.g. `left,right`).
    Enum(Vec<String>),
    /// The setting's numeric (signed integer) value must fall within one of these inclusive
    /// ranges (e.g. `-1,0-9999` -> `[(-1, -1), (0, 9999)]`).
    Range(Vec<(isize, isize)>),
}

impl Constraint {
    /// Parses the `values` field from `settings.json` into a `Constraint`. Returns `None` when the
    /// field is empty. When every comma-separated token parses as an integer or `lo-hi` range, the
    /// result is a [`Constraint::Range`]; otherwise it is a [`Constraint::Enum`] of the raw tokens.
    #[must_use]
    pub fn parse(values: &str) -> Option<Constraint> {
        let tokens: Vec<&str> = values.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if tokens.is_empty() {
            return None;
        }

        let ranges: Option<Vec<(isize, isize)>> = tokens.iter().map(|t| parse_range_token(t)).collect();
        match ranges {
            Some(ranges) => Some(Constraint::Range(ranges)),
            None => Some(Constraint::Enum(tokens.into_iter().map(str::to_string).collect())),
        }
    }

    /// Returns the constraint's allowed values as individual tokens, e.g. `["left", "right"]` or
    /// `["-1", "0-9999"]`.
    #[must_use]
    pub fn tokens(&self) -> Vec<String> {
        match self {
            Constraint::Enum(values) => values.clone(),
            Constraint::Range(ranges) => ranges
                .iter()
                .map(|(lo, hi)| if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") })
                .collect(),
        }
    }

    /// Compact one-line form for tables, e.g. `left | right` or `-1 | 0-9999`.
    #[must_use]
    pub fn compact(&self) -> String {
        self.tokens().join(" | ")
    }

    /// Returns true when the given value satisfies the constraint.
    #[must_use]
    pub fn allows(&self, value: &Setting) -> bool {
        match self {
            Constraint::Enum(allowed) => {
                let v = value.value_string();
                allowed.iter().any(|a| a == &v)
            }
            Constraint::Range(ranges) => {
                let n = value.to_sint();
                ranges.iter().any(|(lo, hi)| n >= *lo && n <= *hi)
            }
        }
    }
}

impl Display for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "one of: {}", self.tokens().join(", "))
    }
}

/// Parses a single token as either an integer literal (`-1` -> `(-1, -1)`) or an inclusive
/// `lo-hi` range (`0-9999` -> `(0, 9999)`, negatives allowed on either side). Returns `None`
/// when the token is not numeric.
fn parse_range_token(token: &str) -> Option<(isize, isize)> {
    if let Ok(n) = token.parse::<isize>() {
        return Some((n, n));
    }

    // Find a '-' separator that is not the leading sign of `lo`.
    let pos = token[1..].find('-').map(|p| p + 1)?;
    let lo = token[..pos].parse::<isize>().ok()?;
    let hi = token[pos + 1..].parse::<isize>().ok()?;
    Some((lo, hi))
}

/// `SettingInfo` returns information about a given setting
#[derive(Clone, PartialEq, Debug)]
pub struct SettingInfo {
    /// Name of the key (dot notation, e.g. `dns.resolver.enabled`)
    pub key: String,
    /// Description of the setting
    pub description: String,
    /// Default setting if none has been specified
    pub default: Setting,
    /// Optional constraint restricting the values this setting may take.
    pub constraint: Option<Constraint>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn setting() {
        let s = Setting::from_str("b:true").unwrap();
        assert_eq!(s, Setting::Bool(true));
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());
        assert_eq!("true", s.to_string());
        assert_eq!(vec!("true"), s.to_map());

        let s = Setting::from_str("i:-1").unwrap();
        assert_eq!(s, Setting::SInt(-1));
        assert!(s.to_bool());
        assert_eq!(-1, s.to_sint());
        assert_eq!(18446744073709551615, s.to_uint());
        assert_eq!("-1", s.to_string());
        assert_eq!(vec!("-1"), s.to_map());

        let s = Setting::from_str("i:1").unwrap();
        assert_eq!(s, Setting::SInt(1));
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());
        assert_eq!("1", s.to_string());
        assert_eq!(vec!("1"), s.to_map());

        let s = Setting::from_str("s:hello world").unwrap();
        assert_eq!(s, Setting::String("hello world".into()));
        assert!(!s.to_bool());
        assert_eq!(0, s.to_sint());
        assert_eq!(0, s.to_uint());
        assert_eq!("hello world", s.to_string());
        assert_eq!(vec!("hello world"), s.to_map());

        let s = Setting::from_str("m:foo,bar,baz").unwrap();
        assert_eq!(s, Setting::Map(vec!["foo".into(), "bar".into(), "baz".into()]));
        assert!(s.to_bool());
        assert_eq!(3, s.to_sint());
        assert_eq!(3, s.to_uint());
        assert_eq!("foo,bar,baz", s.to_string());
        assert_eq!(vec!["foo", "bar", "baz"], s.to_map());

        let s = Setting::from_str("notexist:true");
        assert!(matches!(s, Err(Error::Config(_))));

        let s = Setting::from_str("b:foobar");
        assert!(matches!(s, Err(Error::Config(_))));

        let s = Setting::from_str("i:foobar");
        assert!(matches!(s, Err(Error::Config(_))));

        let s = Setting::from_str("u:-1");
        assert!(matches!(s, Err(Error::Config(_))));

        let s = Setting::from_str("s:true").unwrap();
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());

        let s = Setting::from_str("s:false").unwrap();
        assert!(!s.to_bool());
        assert_eq!(0, s.to_sint());
        assert_eq!(0, s.to_uint());

        let s = Setting::from_str("s:1").unwrap();
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());

        let s = Setting::from_str("s:0").unwrap();
        assert!(!s.to_bool());
        assert_eq!(0, s.to_sint());
        assert_eq!(0, s.to_uint());

        let s = Setting::from_str("s:on").unwrap();
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());

        let s = Setting::from_str("s:yes").unwrap();
        assert!(s.to_bool());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());

        let s = Setting::from_str("s:off").unwrap();
        assert!(!s.to_bool());
        assert_eq!(0, s.to_sint());
        assert_eq!(0, s.to_uint());
    }

    #[test]
    fn float_setting() {
        let s = Setting::from_str("f:1.5").unwrap();
        assert_eq!(s, Setting::Float(1.5));
        assert!(s.to_bool());
        assert_eq!(1.5, s.to_float());
        assert_eq!(1, s.to_sint());
        assert_eq!(1, s.to_uint());
        assert_eq!("1.5", s.to_string());

        let s = Setting::from_str("f:0").unwrap();
        assert_eq!(s, Setting::Float(0.0));
        assert!(!s.to_bool());
        assert_eq!(0.0, s.to_float());

        let s = Setting::from_str("f:-2.25").unwrap();
        assert_eq!(s, Setting::Float(-2.25));
        assert_eq!(-2.25, s.to_float());

        // cross-type coercions
        assert_eq!(3.0, Setting::UInt(3).to_float());
        assert_eq!(-7.0, Setting::SInt(-7).to_float());
        assert_eq!(1.0, Setting::Bool(true).to_float());

        assert!(Setting::from_str("f:notafloat").is_err());
    }

    #[test]
    fn constraint_enum() {
        let c = Constraint::parse("left,right").unwrap();
        assert_eq!(c, Constraint::Enum(vec!["left".into(), "right".into()]));
        assert!(c.allows(&Setting::Map(vec!["left".into()])));
        assert!(c.allows(&Setting::String("right".into())));
        assert!(!c.allows(&Setting::Map(vec!["middle".into()])));
    }

    #[test]
    fn constraint_range() {
        let c = Constraint::parse("-1,0-9999").unwrap();
        assert_eq!(c, Constraint::Range(vec![(-1, -1), (0, 9999)]));
        assert!(c.allows(&Setting::SInt(-1)));
        assert!(c.allows(&Setting::SInt(0)));
        assert!(c.allows(&Setting::SInt(9999)));
        assert!(!c.allows(&Setting::SInt(-2)));
        assert!(!c.allows(&Setting::SInt(10000)));
    }

    #[test]
    fn constraint_parse_edge_cases() {
        assert_eq!(Constraint::parse(""), None);
        // negative range bounds on both sides
        assert_eq!(Constraint::parse("-5--1"), Some(Constraint::Range(vec![(-5, -1)])));
    }
}

#[cfg(test)]
mod round_trip_tests {
    use super::*;

    #[test]
    fn display_round_trip() {
        for wire in [
            "b:true",
            "b:false",
            "i:-1",
            "i:42",
            "u:0",
            "u:9999",
            "f:1.5",
            "f:-2.25",
            "f:0",
            "s:hello world",
            "m:foo,bar,baz",
            "m:",
        ] {
            let s = Setting::from_str(wire).unwrap();
            assert_eq!(format!("{s}"), wire, "Display round-trip failed for {wire}");
            let s2 = Setting::from_str(&format!("{s}")).unwrap();
            assert_eq!(s, s2, "from_str(Display(s)) != s for {wire}");
        }
    }

    #[test]
    fn serde_round_trip() {
        for wire in ["b:true", "i:-42", "u:100", "f:3.14", "s:hello", "m:x,y,z", "m:"] {
            let s = Setting::from_str(wire).unwrap();
            let serialized = serde_json::to_string(&s).unwrap();
            let deserialized: Setting = serde_json::from_str(&serialized).unwrap();
            assert_eq!(s, deserialized, "serde round-trip failed for {wire}");
        }
    }
}
