//! Downloads and merges the CSS definitions extracted by w3c/webref: property
//! grammars, value types, at-rules, and selectors from the W3C editor's-draft
//! specs (curated branch).

use crate::types::{AtRuleValue, Selector};
use anyhow::{Context, Result};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const REPO: &str = "w3c/webref";
const LOCATION: &str = "ed/css";
const BRANCH: &str = "curated";
pub const CACHE_DIR: &str = ".css_cache";

#[derive(Debug, Deserialize)]
pub struct DirectoryListItem {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(rename = "type")]
    pub item_type: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct WebRefValue {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "value")]
    pub syntax: String,
    #[serde(default, rename = "type")]
    pub value_type: String,
    /// Additional accompanied values
    #[serde(default)]
    pub values: Vec<WebRefValue>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct WebRefProperty {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "value")]
    pub syntax: String,
    #[serde(default, rename = "newValues")]
    pub new_syntax: String,
    /// Additional accompanied values for this property
    #[serde(default)]
    pub values: Vec<WebRefValue>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct WebRefAtRule {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub descriptors: Vec<WebRefAtRuleDescriptor>,
    #[serde(default, rename = "value")]
    pub syntax: String,
    #[serde(default)]
    pub values: Option<Vec<AtRuleValue>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct WebRefAtRuleDescriptor {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "value")]
    pub syntax: String,
    #[serde(default)]
    pub initial: String,
}

/// One webref spec extract file (e.g. `css-backgrounds.json`).
#[derive(Debug, Default, Deserialize)]
struct WebRefFileData {
    #[serde(default)]
    properties: Vec<WebRefProperty>,
    #[serde(default)]
    values: Vec<WebRefValue>,
    #[serde(default)]
    atrules: Vec<WebRefAtRule>,
    #[serde(default)]
    selectors: Vec<Selector>,
}

#[derive(Debug, Default)]
pub struct WebRefData {
    pub properties: Vec<WebRefProperty>,
    pub values: Vec<WebRefValue>,
    pub at_rules: Vec<WebRefAtRule>,
    pub selectors: Vec<Selector>,
}

#[derive(Debug, Default)]
struct ParseData {
    properties: BTreeMap<String, WebRefProperty>,
    values: BTreeMap<String, WebRefValue>,
    at_rules: BTreeMap<String, WebRefAtRule>,
    selectors: BTreeMap<String, Selector>,
}

pub fn get_webref_data(client: &reqwest::blocking::Client) -> Result<WebRefData> {
    let files = get_webref_files(client)?;

    let mut pd = ParseData::default();

    for file in &files {
        if file.item_type != "file" || !file.name.ends_with(".json") {
            continue;
        }

        let shortname = file.name.trim_end_matches(".json");
        // Spec extracts come in an unversioned form plus per-level snapshots
        // (css-backgrounds.json, css-backgrounds-4.json, ...); only the
        // unversioned one carries the full, current definitions.
        if shortname.chars().last().is_some_and(|c| c.is_ascii_digit()) {
            eprintln!("Skipping versioned spec {shortname}");
            continue;
        }

        let content = download_file_content(client, file).with_context(|| format!("downloading {}", file.path))?;
        decode_file_content(&content, &mut pd).with_context(|| format!("parsing {}", file.name))?;
    }

    Ok(WebRefData {
        properties: pd.properties.into_values().collect(),
        values: pd.values.into_values().collect(),
        at_rules: pd.at_rules.into_values().collect(),
        selectors: pd.selectors.into_values().collect(),
    })
}

fn get_webref_files(client: &reqwest::blocking::Client) -> Result<Vec<DirectoryListItem>> {
    let url = format!("https://api.github.com/repos/{REPO}/contents/{LOCATION}?ref={BRANCH}");
    let resp = client.get(&url).send()?.error_for_status()?;
    let body = resp.bytes()?;
    serde_json::from_slice(&body).context("parsing webref directory listing")
}

/// Returns the file's content, from the local cache when it still matches the
/// upstream git blob SHA, downloading and re-caching it otherwise.
fn download_file_content(client: &reqwest::blocking::Client, file: &DirectoryListItem) -> Result<Vec<u8>> {
    let cache_path = Path::new(CACHE_DIR).join("specs").join(&file.name);
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Ok(content) = fs::read(&cache_path) {
        if compute_git_blob_sha1(&content) == file.sha {
            return Ok(content);
        }
    }

    eprintln!("Cache file is outdated, downloading {}", file.path);
    let url = file
        .download_url
        .as_deref()
        .context("listing entry has no download_url")?;
    let resp = client.get(url).send()?.error_for_status()?;
    let body = resp.bytes()?.to_vec();
    fs::write(&cache_path, &body).with_context(|| format!("writing cache file {}", cache_path.display()))?;

    Ok(body)
}

/// Git blob SHA-1 (`sha1("blob <len>\0<content>")`), used to validate the
/// cache against the GitHub directory listing.
fn compute_git_blob_sha1(content: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()).as_bytes());
    hasher.update(content);
    let mut out = String::with_capacity(40);
    for byte in hasher.finalize() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn decode_file_content(content: &[u8], pd: &mut ParseData) -> Result<()> {
    let file_data: WebRefFileData = serde_json::from_slice(content)?;

    for property in file_data.properties {
        for v in &property.values {
            process_value(&v.name, &v.value_type, &v.syntax, pd);
            process_extra_values(&v.values, pd);
        }

        if let Some(existing) = pd.properties.get(&property.name) {
            let mut p = existing.clone();

            if p.syntax.is_empty() {
                p.syntax = property.syntax.clone();
            } else if p.syntax != property.syntax && !property.syntax.is_empty() {
                eprintln!("Different syntax for duplicated property {}", property.name);
                eprintln!("Old: {}", p.syntax);
                eprintln!("New: {}", property.syntax);
            }

            // `newValues` entries (a spec extending another spec's property)
            // are folded into the base grammar as extra alternatives.
            if !p.new_syntax.is_empty() && !p.syntax.is_empty() {
                p.syntax = format!("{} | {}", p.syntax, p.new_syntax);
                p.new_syntax = String::new();
            }

            if !property.new_syntax.is_empty() {
                if !p.syntax.is_empty() {
                    p.syntax = format!("{} | {}", p.syntax, property.new_syntax);
                } else if !p.new_syntax.is_empty() {
                    p.new_syntax = format!("{} | {}", p.new_syntax, property.new_syntax);
                } else {
                    p.new_syntax = property.new_syntax.clone();
                }
            }

            pd.properties.insert(p.name.clone(), p);
            continue;
        }

        pd.properties.insert(property.name.clone(), property);
    }

    process_extra_values(&file_data.values, pd);

    for at_rule in file_data.atrules {
        if let Some(existing) = pd.at_rules.get(&at_rule.name) {
            let mut a = existing.clone();

            if a.syntax.is_empty() {
                a.syntax = at_rule.syntax.clone();
            }

            if !a.syntax.is_empty() && !at_rule.syntax.is_empty() && a.syntax != at_rule.syntax {
                eprintln!("Different syntax for duplicated at-rule {}", at_rule.name);
                eprintln!("Old: {}", a.syntax);
                eprintln!("New: {}", at_rule.syntax);
            }

            if let Some(values) = at_rule.values {
                if !values.is_empty() {
                    a.values.get_or_insert_with(Vec::new).extend(values);
                }
            }
            a.descriptors.extend(at_rule.descriptors);

            pd.at_rules.insert(a.name.clone(), a);
            continue;
        }

        pd.at_rules.insert(at_rule.name.clone(), at_rule);
    }

    for selector in file_data.selectors {
        pd.selectors.insert(selector.name.clone(), selector);
    }

    Ok(())
}

/// Process a single value (from either root values or property values) and add
/// it to the ParseData if possible.
fn process_value(name: &str, value_type: &str, syntax: &str, pd: &mut ParseData) {
    if name == syntax {
        return;
    }

    let syntax = quote_parentheses(syntax);

    // If the value already exists, update the syntax if possible
    if let Some(existing) = pd.values.get(name) {
        let mut v = existing.clone();

        if v.syntax.is_empty() {
            v.syntax = syntax.clone();
        }

        // Skip built-in values (<integer> has syntax "<integer>", which
        // results in a loop when resolving)
        if v.syntax == v.name {
            eprintln!("name == syntax, skipping as this is an built-in value: {name}");
            return;
        }

        // Not all values have the same syntax. It can change. We ignore this
        // and keep the first one we saw.
        if !v.syntax.is_empty() && !syntax.is_empty() && v.syntax != syntax {
            eprintln!("Different syntax for duplicated value {name}");
            eprintln!("Old: {}", v.syntax);
            eprintln!("New: {syntax}");
        }

        pd.values.insert(name.to_string(), v);
        return;
    }

    if value_type == "value" {
        eprintln!("value type. Skipping: {name}");
        return;
    }

    // Skip <integer> = syntax("<integer>")
    if syntax == name {
        eprintln!("value==name. Skipping: {name}");
        return;
    }

    if syntax.is_empty() {
        eprintln!("empty value/syntax: {name}");
        return;
    }

    pd.values.insert(
        name.to_string(),
        WebRefValue {
            name: name.to_string(),
            syntax,
            value_type: String::new(),
            values: Vec::new(),
        },
    );
}

fn process_extra_values(values: &[WebRefValue], pd: &mut ParseData) {
    for value in values {
        process_value(&value.name, &value.value_type, &value.syntax, pd);
        process_extra_values(&value.values, pd);
    }
}

/// Wraps literal punctuation in a grammar in quotes (`{` -> `'{'`) so it can't
/// be confused with value-definition-syntax metacharacters. A `(` keeps track
/// of whether it was quoted so the matching `)` is treated the same way:
/// function-call parens (`calc(`) stay bare, standalone parens get quoted.
fn quote_parentheses(input: &str) -> String {
    let runes: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len());
    let mut quote_closing_parenthesis: Vec<bool> = Vec::new();

    for i in 0..runes.len() {
        let current = runes[i];

        let mut found = false;
        for literal in ['{', '}', ':', ';', '$', ','] {
            if current == literal && (i == 0 || runes[i - 1].is_whitespace()) {
                result.push('\'');
                result.push(current);
                result.push('\'');
                found = true;
            }
        }
        if found {
            continue;
        }

        if current == '(' {
            if i == 0 || runes[i - 1].is_whitespace() {
                result.push_str("'('");
                quote_closing_parenthesis.push(true);
                continue;
            }

            if runes[i - 1].is_alphabetic() || runes[i - 1].is_ascii_digit() {
                result.push('(');
                quote_closing_parenthesis.push(false);
                continue;
            }
            // Any other preceding character: fall through and emit the paren
            // bare without a stack entry, exactly like the Go tool did.
        }

        if current == ')' {
            let quote_parenthesis = quote_closing_parenthesis.pop().unwrap_or(true);
            if quote_parenthesis {
                result.push_str("')'");
            } else {
                result.push(')');
            }
            continue;
        }

        result.push(current);
    }

    result
}
