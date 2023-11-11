use core::fmt::Display;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

impl Serialize for Setting {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = self.to_string();
        serializer.collect_str(&s)
    }
}

impl<'de> Deserialize<'de> for Setting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        match Setting::from_string(value.as_str()) {
            None => Err(serde::de::Error::custom("Cannot deserialize")),
            Some(setting) => Ok(setting),
        }
    }
}

impl Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setting::SInt(value) => write!(f, "i:{}", value),
            Setting::UInt(value) => write!(f, "u:{}", value),
            Setting::String(value) => write!(f, "s:{}", value),
            Setting::Bool(value) => write!(f, "b:{}", value),
            Setting::Map(values) => {
                let mut result = String::new();
                for value in values {
                    result.push_str(value);
                    result.push(',');
                }
                result.pop();
                write!(f, "m: {}", result)
            }
        }
    }
}

impl Setting {
    // first element is the type:
    //   b:true
    //   i:-123
    //   u:234
    //   s:hello world
    //   m:foo,bar,baz

    /// Converts a string to a setting or None when the string is invalid
    pub fn from_string(key: &str) -> Option<Setting> {
        let (key_type, key_value) = key.split_once(':').unwrap();

        match key_type {
            "b" => match key_value.parse::<bool>() {
                Ok(value) => Some(Setting::Bool(value)),
                Err(_) => None,
            },
            "i" => match key_value.parse::<isize>() {
                Ok(value) => Some(Setting::SInt(value)),
                Err(_) => None,
            },
            "u" => match key_value.parse::<usize>() {
                Ok(value) => Some(Setting::UInt(value)),
                Err(_) => None,
            },
            "s" => Some(Setting::String(key_value.to_string())),
            "m" => {
                let mut values = Vec::new();
                for value in key_value.split(',') {
                    values.push(value.to_string());
                }
                Some(Setting::Map(values))
            }
            _ => None,
        }
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
        let s = Setting::from_string("b:true");
        assert_eq!(s, Some(Setting::Bool(true)));

        let s = Setting::from_string("i:-1");
        assert_eq!(s, Some(Setting::SInt(-1)));

        let s = Setting::from_string("i:1");
        assert_eq!(s, Some(Setting::SInt(1)));

        let s = Setting::from_string("s:hello world");
        assert_eq!(s, Some(Setting::String("hello world".into())));

        let s = Setting::from_string("m:foo,bar,baz");
        assert_eq!(
            s,
            Some(Setting::Map(vec!["foo".into(), "bar".into(), "baz".into()]))
        );

        let s = Setting::from_string("notexist:true");
        assert_eq!(s, None);

        let s = Setting::from_string("b:foobar");
        assert_eq!(s, None);

        let s = Setting::from_string("i:foobar");
        assert_eq!(s, None);

        let s = Setting::from_string("u:-1");
        assert_eq!(s, None);
    }
}
