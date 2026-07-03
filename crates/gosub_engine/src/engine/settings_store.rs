//! The engine's built-in settings schema.
//!
//! `gosub_config` provides the configuration *machinery* but is agnostic of which settings exist.
//! The engine owns the schema: the set of known keys with their types, defaults, constraints and
//! descriptions. It lives in two embedded files — `settings.json` (engine settings) and
//! `useragent-settings.json` (user-agent settings, merged under the `useragent` namespace) — which
//! are parsed here into the [`SettingInfo`] lists that seed a [`Config`].

use gosub_config::settings::{Constraint, Setting, SettingInfo};
use gosub_config::Config;
use serde::Deserialize;
use serde_json::Value;
use std::str::FromStr;

/// The engine's own settings schema (network, rendering, …), embedded for easy editing.
const SETTINGS_JSON: &str = include_str!("settings.json");

/// The user-agent / client settings schema. Defined with keys *relative* to the user agent (e.g.
/// `tab.close_button`); merged into the engine config under the `useragent` namespace, becoming
/// `useragent.tab.close_button`. Kept here for now; intended to move to a dedicated client crate,
/// which would then `merge` it in itself.
const USERAGENT_SETTINGS_JSON: &str = include_str!("useragent-settings.json");

/// Namespace the user-agent settings are merged under.
const USERAGENT_NAMESPACE: &str = "useragent";

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
    let config = Config::new(schema_from(SETTINGS_JSON, "settings.json"));

    // User-agent settings live in their own schema and are folded in under a namespace.
    let user_agent = Config::new(schema_from(USERAGENT_SETTINGS_JSON, "useragent-settings.json"));
    config.merge(&user_agent, USERAGENT_NAMESPACE);

    config
}

/// Parses an embedded schema file, logging and returning an empty schema on failure (the files are
/// validated by tests, so this never happens in practice).
fn schema_from(json: &str, name: &str) -> Vec<SettingInfo> {
    parse_schema(json).unwrap_or_else(|err| {
        log::error!("built-in {name} is invalid, skipping it: {err}");
        Vec::new()
    })
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
        // Engine-owned settings.
        assert!(cfg.has("dns.local.enabled"));
        assert_eq!(cfg.get_uint("dns.cache.max_entries"), 1000);
        assert!(cfg.get_bool("renderer.opengl.enabled"));
        // User-agent settings, merged under the `useragent` namespace.
        assert_eq!(cfg.get_string("useragent.general.default_page"), "about:blank");
        assert!(cfg.has("useragent.tab.close_button"));
    }

    #[test]
    fn schema_constraints_are_applied() {
        let cfg = default_config();
        // `useragent.tab.close_button` is constrained to `left,right` (constraint survives the merge).
        assert!(cfg
            .set("useragent.tab.close_button", Setting::Map(vec!["right".into()]))
            .is_ok());
        assert!(cfg
            .set("useragent.tab.close_button", Setting::Map(vec!["nope".into()]))
            .is_err());
    }
}
