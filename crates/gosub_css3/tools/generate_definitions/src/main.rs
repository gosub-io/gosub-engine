//! Generates the CSS definition JSON files embedded in gosub_css3
//! (`resources/definitions/`) by merging webref's spec grammars with MDN's
//! property metadata. See README.md for the full data-flow description.

mod mdn;
mod types;
mod webref;

use anyhow::Result;
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use types::{AtRule, AtRuleDescriptor, Data, Property, Value};

const RESOURCE_PATH: &str = ".output/definitions";
const MULTI_FILE_PREFIX: &str = "definitions_";

/// Removes a value-definition-syntax comma multiplier (`#`, optionally bounded
/// as `#{min,max}`) from the very end of a grammar, turning a comma-separated
/// list grammar into its single-value form.
fn strip_trailing_comma_multiplier(re: &Regex, syntax: &str) -> String {
    re.replace_all(syntax.trim_end_matches(' '), "").into_owned()
}

fn main() -> Result<()> {
    // A value-definition-syntax comma multiplier at the very end of a grammar.
    let trailing_comma_multiplier = Regex::new(r"#(\{[0-9]+(,[0-9]*)?\})?\s*$")?;

    // The "optional comma-list, then a mandatory comma, then a final term"
    // shorthand that MDN and webref both use to flatten a repeated layer group
    // onto one line (e.g. `background = <bg-layer>#? , <final-bg-layer>`).
    // It is rewritten into the spec's actual grammar, where the separating
    // comma lives inside the repeat: `[ <X> , ]* <Y>`. The linearized form
    // makes the comma mandatory, so a single final term (`background: red`)
    // fails to match; keeping the comma inside the repeat matches both one
    // and many layers.
    let comma_list_idiom = Regex::new(r"(<[^>]+>)#\? , ")?;

    let client = reqwest::blocking::Client::builder()
        .user_agent("gosub-generate-definitions")
        .build()?;

    let webref_data = webref::get_webref_data(&client)?;
    let mdn_data = mdn::get_mdn_data(&client)?;

    let mut data = Data::default();

    eprintln!(
        "Webref data: {} properties, {} values, {} at-rules, {} selectors",
        webref_data.properties.len(),
        webref_data.values.len(),
        webref_data.at_rules.len(),
        webref_data.selectors.len(),
    );

    // Index webref properties by name so we can source authoritative grammar
    // (syntax) from the W3C specs. webref is standards-scoped and does not
    // cover vendor-prefixed or legacy properties.
    let webref_by_name: std::collections::BTreeMap<&str, &webref::WebRefProperty> =
        webref_data.properties.iter().map(|p| (p.name.as_str(), p)).collect();

    // MDN is the authoritative property SET: it tracks the full shipping
    // surface including vendor-prefixed and legacy properties that webref
    // omits. For each property we prefer webref's spec grammar for the syntax,
    // falling back to MDN's syntax when webref has no entry for it.
    for (name, mdn_prop) in &mdn_data {
        let mut syntax = mdn_prop.syntax.clone();
        if let Some(webref_prop) = webref_by_name.get(name.as_str()) {
            if !webref_prop.syntax.is_empty() {
                syntax = webref_prop.syntax.clone();
            }
        }

        let syntax = comma_list_idiom.replace_all(&syntax, "[ ${1} , ]* ").into_owned();

        let computed = if mdn_prop.computed.array.is_empty() {
            if !mdn_prop.computed.string.is_empty() {
                vec![mdn_prop.computed.string.clone()]
            } else {
                Vec::new()
            }
        } else {
            mdn_prop.computed.array.clone()
        };

        data.properties.push(Property {
            name: name.clone(),
            syntax,
            computed,
            initial: mdn_prop.initial.clone(),
            inherited: mdn_prop.inherited,
        });
    }

    for value in &webref_data.values {
        data.values.push(Value {
            name: value.name.clone(),
            syntax: value.syntax.clone(),
        });
    }

    // Value definitions are named "<name>" in the output; track them by that form.
    let mut defined_values: BTreeSet<String> = data.values.iter().map(|v| v.name.clone()).collect();

    // Backfill 1: MDN's syntaxes.json is a value-type dictionary webref does
    // not fully cover (e.g. outline-radius, single-animation-*). Add every
    // entry webref did not already define, so grammar references to them
    // resolve.
    for (name, syntax) in mdn::get_mdn_syntaxes(&client)? {
        let key = format!("<{name}>");
        if syntax.is_empty() || defined_values.contains(&key) {
            continue;
        }
        data.values.push(Value {
            name: key.clone(),
            syntax,
        });
        defined_values.insert(key);
    }

    // Backfill 2: webref decomposes some shorthands into sub-properties it
    // then references as value types (e.g. box-shadow -> <spread-shadow>,
    // which uses <'box-shadow-blur'>). Those sub-properties live in webref but
    // not in MDN's property set, so they are absent from data.properties. Emit
    // any webref property that some grammar references but that is otherwise
    // undefined, as a value-type definition sourced from its webref grammar.
    let mdn_prop_set: BTreeSet<&str> = data.properties.iter().map(|p| p.name.as_str()).collect();

    let mut corpus = String::new();
    for prop in &data.properties {
        corpus.push_str(&prop.syntax);
        corpus.push('\n');
    }
    for value in &data.values {
        corpus.push_str(&value.syntax);
        corpus.push('\n');
    }

    for wp in &webref_data.properties {
        let key = format!("<{}>", wp.name);
        if wp.syntax.is_empty() || mdn_prop_set.contains(wp.name.as_str()) || defined_values.contains(&key) {
            continue;
        }
        // Only capture it when a grammar actually references it as a value
        // type, either as <name> or as the property-reference form <'name'>.
        if corpus.contains(&key) || corpus.contains(&format!("<'{}'>", wp.name)) {
            // A standalone property may be comma-separated (a trailing `#`),
            // but when it is embedded as a value type in another grammar it
            // stands for a single value (e.g. one shadow's <box-shadow-color>
            // inside <spread-shadow>). Keeping the `#` makes that inner list
            // greedily consume the separator comma of the outer list. Drop the
            // trailing comma multiplier.
            data.values.push(Value {
                name: key.clone(),
                syntax: strip_trailing_comma_multiplier(&trailing_comma_multiplier, &wp.syntax),
            });
            defined_values.insert(key);
        }
    }

    for at_rule in &webref_data.at_rules {
        let mut descriptors = Vec::with_capacity(at_rule.descriptors.len());

        for descriptor in &at_rule.descriptors {
            let mut initial = descriptor.initial.clone();
            // Remove "n/a" or "N/A" initial values. This is a faithful port of
            // the Go tool's check, operator-precedence bug included: it also
            // clears any initial whose first byte is 'n' (e.g. "normal",
            // "none") or whose third byte is 'A'.
            let b = initial.as_bytes();
            if b.len() >= 3 && (b[0] == b'n' || (b[0] == b'N' && b[1] == b'/' && b[2] == b'a') || b[2] == b'A') {
                initial = String::new();
            }

            descriptors.push(AtRuleDescriptor {
                name: descriptor.name.clone(),
                syntax: descriptor.syntax.clone(),
                initial,
            });
        }

        data.atrules.push(AtRule {
            name: at_rule.name.clone(),
            descriptors,
            values: at_rule.values.clone(),
        });
    }

    data.selectors = webref_data.selectors.clone();

    eprintln!(
        "Collected data: {} properties, {} values, {} at-rules, {} selectors",
        data.properties.len(),
        data.values.len(),
        data.atrules.len(),
        data.selectors.len(),
    );

    // Sort elements, so that the output is deterministic and we have less
    // issues with version control
    data.properties.sort_by(|a, b| a.name.cmp(&b.name));
    data.values.sort_by(|a, b| a.name.cmp(&b.name));
    data.atrules.sort_by(|a, b| a.name.cmp(&b.name));
    // webref does not emit at-rule descriptors/values in a stable order, so
    // sort the nested collections too; otherwise every regeneration produces
    // spurious churn.
    for at_rule in &mut data.atrules {
        at_rule.descriptors.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(values) = &mut at_rule.values {
            // Merging specs can leave duplicate names (e.g. @media has two
            // "all" entries); tie-break on value and nested-list size so the
            // order does not depend on which spec file was processed first.
            values.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.value.cmp(&b.value))
                    .then_with(|| a.values.as_ref().map(Vec::len).cmp(&b.values.as_ref().map(Vec::len)))
            });
            for value in values {
                if let Some(entries) = &mut value.values {
                    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.value.cmp(&b.value)));
                }
            }
        }
    }
    data.selectors.sort_by(|a, b| a.name.cmp(&b.name));

    export_multi_file(&data)?;
    export_single_file(&data)?;

    Ok(())
}

fn export_single_file(data: &Data) -> Result<()> {
    export_data(data, &Path::new(RESOURCE_PATH).join("definitions.json"))
}

fn export_multi_file(data: &Data) -> Result<()> {
    let dir = Path::new(RESOURCE_PATH);
    fs::create_dir_all(dir)?;

    export_data(
        &data.properties,
        &dir.join(format!("{MULTI_FILE_PREFIX}properties.json")),
    )?;
    export_data(&data.values, &dir.join(format!("{MULTI_FILE_PREFIX}values.json")))?;
    export_data(&data.atrules, &dir.join(format!("{MULTI_FILE_PREFIX}at-rules.json")))?;
    export_data(&data.selectors, &dir.join(format!("{MULTI_FILE_PREFIX}selectors.json")))?;

    Ok(())
}

fn export_data<T: serde::Serialize>(data: &T, path: &Path) -> Result<()> {
    let mut out = serde_json::to_vec_pretty(data)?;
    out.push(b'\n');
    fs::write(path, out)?;
    Ok(())
}
