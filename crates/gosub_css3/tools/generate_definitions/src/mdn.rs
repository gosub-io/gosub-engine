//! Fetches MDN's CSS data (mdn/data): the authoritative set of shipping
//! properties (including vendor-prefixed and legacy ones webref omits) and its
//! value-type grammar dictionary.

use crate::types::StringMaybeArray;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

const MDN_PROPERTIES: &str = "https://raw.githubusercontent.com/mdn/data/main/css/properties.json";
const MDN_SYNTAXES: &str = "https://raw.githubusercontent.com/mdn/data/main/css/syntaxes.json";

#[derive(Debug, Deserialize)]
pub struct MdnItem {
    #[serde(default)]
    pub syntax: String,
    #[serde(default)]
    pub initial: StringMaybeArray,
    #[serde(default)]
    pub computed: StringMaybeArray,
    #[serde(default)]
    pub inherited: bool,
}

#[derive(Debug, Deserialize)]
struct MdnSyntax {
    #[serde(default)]
    syntax: String,
}

pub fn get_mdn_data() -> Result<BTreeMap<String, MdnItem>> {
    let body = crate::fetch::get(MDN_PROPERTIES)?;
    serde_json::from_slice(&body).context("parsing MDN properties.json")
}

/// Returns MDN's value-type dictionary (css/syntaxes.json) as a map of type
/// name (without angle brackets) to its grammar. webref does not fully cover
/// these value types, so they are used to backfill value definitions.
pub fn get_mdn_syntaxes() -> Result<BTreeMap<String, String>> {
    let body = crate::fetch::get(MDN_SYNTAXES)?;
    let raw: BTreeMap<String, MdnSyntax> = serde_json::from_slice(&body).context("parsing MDN syntaxes.json")?;

    Ok(raw.into_iter().map(|(name, item)| (name, item.syntax)).collect())
}
