use crate::errors::Error;
use core::fmt::Display;
use log::warn;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

/// A setting can be either a signed integer, unsigned integer, string, map or boolean.
/// Maps could be created by using comma separated strings maybe
#[derive(Clone, PartialEq, Debug)]
pub enum Setting {
    SInt(isize),
    UInt(usize),
    String(String),
    Bool(bool),
    Map(Vec<String>),
}

impl Setting {
    pub fn to_bool(&self) -> bool {
        if !matches!(self, Setting::Bool(_)) {
            warn!("setting is not a boolean");
        }

        match self {
            Setting::Bool(value) => *value,
            Setting::SInt(value) => *value != 0,
            Setting::UInt(value) => *value != 0,
            Setting::String(value) => is_bool_value(value),
            Setting::Map(values) => !values.is_empty(),
        }
    }

    pub fn to_sint(&self) -> isize {
        if !matches!(self, Setting::SInt(_)) {
            warn!("setting is not an signed integer");
        }

        match self {
            Setting::SInt(value) => *value,
            Setting::UInt(value) => *value as isize,
            Setting::Bool(value) => *value as isize,
            Setting::String(value) => is_bool_value(value) as isize,
            Setting::Map(values) => values.len() as isize,
        }
    }

    pub fn to_uint(&self) -> usize {
        if !matches!(self, Setting::UInt(_)) {
            warn!("setting is not an unsigned integer");
        }

        match self {
            Setting::UInt(value) => *value,
            Setting::SInt(value) => *value as usize,
            Setting::Bool(value) => *value as usize,
            Setting::String(value) => is_bool_value(value) as usize,
            Setting::Map(values) => values.len(),
        }
    }

    #[allow(clippy::inherent_to_string_shadow_display)]
    pub fn to_string(&self) -> String {
        if !matches!(self, Setting::String(_)) {
            warn!("setting is not a string");
        }

        match self {
            Setting::SInt(value) => value.to_string(),
            Setting::UInt(value) => value.to_string(),
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

    pub fn to_map(&self) -> Vec<String> {
        if !matches!(self, Setting::Map(_)) {
            warn!("setting is not a map");
        }

        match self {
            Setting::Map(values) => values.clone(),
            other => vec![other.to_string()],
        }
    }
}

fn is_bool_value(s: &str) -> bool {
    let us = s.to_uppercase();
    if ["YES", "ON", "TRUE", "1"].contains(&us.as_str()) {
        return true;
    }

    false
}

impl Serialize for Setting {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = self.to_string();
        serializer.collect_str(&s)
    }
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Setting::from_str(&value)
            .map_err(|err| serde::de::Error::custom(format!("cannot deserialize: {err}")))
    }
}

impl Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setting::SInt(value) => write!(f, "i:{value}"),
            Setting::UInt(value) => write!(f, "u:{value}"),
            Setting::String(value) => write!(f, "s:{value}"),
            Setting::Bool(value) => write!(f, "b:{value}"),
            Setting::Map(values) => {
                let mut result = String::new();
                for value in values {
                    result.push_str(value);
                    result.push(',');
                }
                result.pop();
                write!(f, "m: {result}")
            }
        }
    }
}

impl FromStr for Setting {
    type Err = Error;

    // first element is the type:
    //   b:true
    //   i:-123
    //   u:234
    //   s:hello world
    //   m:foo,bar,baz

    /// Converts a string to a setting or None when the string is invalid
    fn from_str(key: &str) -> Result<Setting, crate::errors::Error> {
        let (key_type, key_value) = key.split_once(':').expect("");

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
            "s" => Setting::String(key_value.to_string()),

            "m" => {
                let mut values = Vec::new();
                for value in key_value.split(',') {
                    values.push(value.to_string());
                }
                Setting::Map(values)
            }
            _ => return Err(Error::Config(format!("unknown setting: {key_value}"))),
        };

        Ok(setting)
    }
}

/// SettingInfo returns information about a given setting
#[derive(Clone, PartialEq, Debug)]
pub struct SettingInfo {
    /// Name of the key (dot notation, (ie: dns.resolver.enabled
    pub key: String,
    /// Description of the setting
    pub description: String,
    /// Default setting if none has been specified
    pub default: Setting,
    /// Timestamp this setting is last accessed (useful for debugging old/obsolete settings)
    pub last_accessed: u64,
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
        assert_eq!(
            s,
            Setting::Map(vec!["foo".into(), "bar".into(), "baz".into()])
        );
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
}
