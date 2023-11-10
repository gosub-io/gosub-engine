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
    // first element is the type:
    //   b:true
    //   i:-123
    //   u:234
    //   s:hello world
    //   m:foo,bar,baz

    /// Converts a string to a setting or None when the string is invalid
    pub fn from_string(p0: &str) -> Option<Setting> {
        let mut parts = p0.splitn(2, ':');
        let p1 = parts.next().unwrap();
        let p2 = parts.next().unwrap();

        match p1 {
            "b" => {
                match p2.parse::<bool>() {
                    Ok(value) => Some(Setting::Bool(value)),
                    Err(_) => None
                }
            }
            "i" => {
                match p2.parse::<isize>() {
                    Ok(value) => Some(Setting::SInt(value)),
                    Err(_) => None
                }
            }
            "u" => {
                match p2.parse::<usize>() {
                    Ok(value) => Some(Setting::UInt(value)),
                    Err(_) => None
                }
            }
            "s" => Some(Setting::String(p2.to_string())),
            "m" => {
                let mut values = Vec::new();
                for value in p2.split(',') {
                    values.push(value.to_string());
                }
                Some(Setting::Map(values))
            },
            _ => None
        }
    }

    /// Converts a setting to a string representation
    pub fn to_string(&self) -> String {
        match self {
            Setting::SInt(value) => format!("i:{}", value),
            Setting::UInt(value) => format!("u:{}", value),
            Setting::String(value) => format!("s:{}", value),
            Setting::Bool(value) => format!("b:{}", value),
            Setting::Map(values) => {
                let mut result = String::new();
                for value in values {
                    result.push_str(value);
                    result.push(',');
                }
                result.pop();
                format!("m:{}", result)
            }
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
    pub last_accessed: u64
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
        assert_eq!(s, Some(Setting::Map(vec!["foo".into(), "bar".into(), "baz".into()])));

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
