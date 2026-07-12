use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A JSON field that may hold either a string or an array of strings (MDN uses
/// both for `initial` and `computed`). Serializes as the array when non-empty,
/// otherwise as the string — matching the Go tool's `StringMaybeArray`.
#[derive(Debug, Default, Clone)]
pub struct StringMaybeArray {
    pub string: String,
    pub array: Vec<String>,
}

impl Serialize for StringMaybeArray {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if !self.array.is_empty() {
            self.array.serialize(serializer)
        } else {
            self.string.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for StringMaybeArray {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SmaVisitor;

        impl<'de> Visitor<'de> for SmaVisitor {
            type Value = StringMaybeArray;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string or an array of strings")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StringMaybeArray {
                    string: v.to_string(),
                    array: Vec::new(),
                })
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut array = Vec::new();
                while let Some(item) = seq.next_element::<String>()? {
                    array.push(item);
                }
                Ok(StringMaybeArray {
                    string: String::new(),
                    array,
                })
            }
        }

        deserializer.deserialize_any(SmaVisitor)
    }
}

/// The complete generated dataset (`definitions.json`).
#[derive(Debug, Default, Serialize)]
pub struct Data {
    pub properties: Vec<Property>,
    pub values: Vec<Value>,
    pub atrules: Vec<AtRule>,
    pub selectors: Vec<Selector>,
}

#[derive(Debug, Serialize)]
pub struct Property {
    pub name: String,
    pub syntax: String,
    pub computed: Vec<String>,
    pub initial: StringMaybeArray,
    pub inherited: bool,
}

#[derive(Debug, Serialize)]
pub struct Value {
    pub name: String,
    pub syntax: String,
}

// The `values` fields below serialize under the key "Values" and as `null`
// when absent: the Go tool's structs had no json tag on that field, so the
// consumer (gosub_css3) reads the Go field name, and Go marshals nil slices
// as null. `Option<Vec<..>>` keeps the absent-vs-empty distinction intact.

#[derive(Debug, Serialize)]
pub struct AtRule {
    pub name: String,
    pub descriptors: Vec<AtRuleDescriptor>,
    #[serde(rename = "Values")]
    pub values: Option<Vec<AtRuleValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtRuleValue {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    // webref's JSON has this key lowercase; the output key is "Values".
    #[serde(rename = "Values", alias = "values", default)]
    pub values: Option<Vec<AtRuleValueEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtRuleValueEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct AtRuleDescriptor {
    pub name: String,
    pub syntax: String,
    pub initial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selector {
    pub name: String,
}
