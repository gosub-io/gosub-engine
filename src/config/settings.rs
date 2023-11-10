use core::fmt::Display;

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

impl Display for Setting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Setting::SInt(value) => write!(f, "{}", value),
            Setting::UInt(value) => write!(f, "{}", value),
            Setting::String(value) => write!(f, "{}", value),
            Setting::Bool(value) => write!(f, "{}", value),
            Setting::Map(values) => {
                let mut result = String::new();
                for value in values {
                    result.push_str(value);
                    result.push(',');
                }
                result.pop();
                write!(f, "{}", result)
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
    pub fn from_string(p0: &str) -> Option<Setting> {
        let (p1, p2) = p0.split_once(':').unwrap();

        match p1 {
            "b" => match p2.parse::<bool>() {
                Ok(value) => Some(Setting::Bool(value)),
                Err(_) => None,
            },
            "i" => match p2.parse::<isize>() {
                Ok(value) => Some(Setting::SInt(value)),
                Err(_) => None,
            },
            "u" => match p2.parse::<usize>() {
                Ok(value) => Some(Setting::UInt(value)),
                Err(_) => None,
            },
            "s" => Some(Setting::String(p2.to_string())),
            "m" => {
                let mut values = Vec::new();
                for value in p2.split(',') {
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
