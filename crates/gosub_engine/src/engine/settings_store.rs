//! The engine's built-in settings schema.
//!
//! `gosub_config` provides the configuration *machinery* but is agnostic of which settings exist.
//! The engine owns the schema: the set of known keys with their types, defaults, constraints and
//! descriptions. It lives in `settings.json` (embedded at build time) and is parsed here into the
//! [`SettingInfo`] list that seeds a [`Config`].

use gosub_config::settings::{Constraint, Setting, SettingInfo};
use gosub_config::Config;
use serde::Deserialize;
use serde_json::Value;
use std::str::FromStr;

/// The engine's settings schema, embedded for easy editing.
const SETTINGS_JSON: &str = include_str!("settings.json");

/// One entry as written in `settings.json`.
#[derive(Debug, Deserialize)]
struct JsonEntry {
    key: String,
    #[serde(rename = "type")]
    _entry_type: String,
    default: String,
    description: String,
    /// Optional comma-separated list of allowed values or ranges (e.g. `left,right` or `-1,0-9999`).
    #[serde(default)]
    values: Option<String>,
}

/// Builds a [`Config`] (in-memory) seeded with the engine's built-in settings schema.
///
/// The schema is embedded at build time and validated by tests, so parsing never fails in
/// practice; should it ever fail, this logs the error and returns an empty schema rather than
/// aborting engine construction.
#[must_use]
pub fn default_config() -> Config {
    match parse_schema(SETTINGS_JSON) {
        Ok(schema) => Config::new(schema),
        Err(err) => {
            log::error!("built-in settings.json is invalid, starting with an empty config: {err}");
            Config::new(Vec::<SettingInfo>::new())
        }
    }
}

/// Parses the sectioned `settings.json` format into a flat list of [`SettingInfo`]. Each top-level
/// key is a section prefix; the final setting key is `"<section>.<entry.key>"`.
fn parse_schema(json: &str) -> anyhow::Result<Vec<SettingInfo>> {
    let mut schema = Vec::new();

    if let Value::Object(sections) = serde_json::from_str(json)? {
        for (section_prefix, entries) in &sections {
            let entries: Vec<JsonEntry> = serde_json::from_value(entries.clone())?;
            for entry in entries {
                schema.push(SettingInfo {
                    key: format!("{section_prefix}.{}", entry.key),
                    description: entry.description,
                    default: Setting::from_str(&entry.default)?,
                    constraint: entry.values.as_deref().and_then(Constraint::parse),
                });
            }
        }
    }

    Ok(schema)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn schema_parses_and_seeds_config() {
        let cfg = default_config();
        // A few known keys from the built-in schema.
        assert!(cfg.has("dns.local.enabled"));
        assert_eq!(cfg.get_uint("dns.cache.max_entries"), 1000);
        assert_eq!(cfg.get_string("useragent.default_page"), "about:blank");
    }

    #[test]
    fn schema_constraints_are_applied() {
        let cfg = default_config();
        // `useragent.tab.close_button` is constrained to `left,right`.
        assert!(cfg.set("useragent.tab.close_button", Setting::Map(vec!["right".into()])).is_ok());
        assert!(cfg.set("useragent.tab.close_button", Setting::Map(vec!["nope".into()])).is_err());
    }
}
