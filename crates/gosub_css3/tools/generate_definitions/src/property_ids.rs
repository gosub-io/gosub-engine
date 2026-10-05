//! The offline half of this tool: turn the checked-in `definitions_properties.json` into the
//! Rust property-id module `gosub_css3` keys its cascade by.
//!
//! This reads the JSON that the network half already produced and writes a source file. It
//! fetches nothing, so it can be re-run at any time to bring the ids back in step with the
//! data after a regeneration.
//!
//! What the module holds is everything a hot path can answer from the JSON alone: the name, the
//! `inherited` flag, the initial value as the source string, whether a percentage computes to a
//! number, and for a shorthand the properties its `computed` list names. Anything that needs the
//! resolved value grammar - whether the property takes a colour, the range its computed value is
//! clamped to - stays with the `PropertyDefinition`, which is reachable by id.

use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::types::Property;

/// The custom-property entry. Custom properties cascade in a pass of their own and are keyed by
/// their full name, so they get no id.
const CUSTOM_PROPERTY: &str = "--*";

/// The computed-value rules that say a percentage for this property computes to a plain number;
/// mirrors `PropertyDefinition::percentage_is_number`.
const PERCENTAGE_IS_NUMBER: [&str; 2] = ["specifiedValueNumberClipped0To1", "specifiedValueClipped0To1"];

/// One property, as the generator sees it.
struct Entry {
    name: String,
    variant: String,
    inherited: bool,
    initial: Option<String>,
    percentage_is_number: bool,
    /// The `computed` list, for a shorthand: the properties it expands to.
    longhands: Vec<String>,
}

/// A property is a shorthand when its `computed` list names more than one thing, which is what
/// `PropertyDefinition::is_shorthand` says.
fn is_shorthand(property: &Property) -> bool {
    property.computed.len() > 1
}

/// The Rust variant name for a CSS property name: the hyphen-separated words in UpperCamelCase,
/// with the empty leading word of a vendor prefix dropped (`-moz-appearance` is `MozAppearance`).
fn variant_name(name: &str) -> String {
    let mut variant = String::with_capacity(name.len());
    for word in name.split('-').filter(|word| !word.is_empty()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            variant.extend(first.to_uppercase());
            variant.push_str(chars.as_str());
        }
    }
    variant
}

/// The initial value as the source string, or `None` where the JSON gives a shorthand's longhand
/// list rather than a value.
fn initial_source(property: &Property) -> Option<String> {
    if property.initial.array.is_empty() {
        Some(property.initial.string.clone())
    } else {
        None
    }
}

fn build_entries(properties: &[Property]) -> (Vec<Entry>, Vec<Entry>) {
    let mut longhands = Vec::new();
    let mut shorthands = Vec::new();

    for property in properties {
        if property.name == CUSTOM_PROPERTY {
            continue;
        }
        let entry = Entry {
            name: property.name.clone(),
            variant: variant_name(&property.name),
            inherited: property.inherited,
            initial: initial_source(property),
            percentage_is_number: property
                .computed
                .iter()
                .any(|rule| PERCENTAGE_IS_NUMBER.contains(&rule.as_str())),
            longhands: if is_shorthand(property) {
                property.computed.clone()
            } else {
                Vec::new()
            },
        };
        if is_shorthand(property) {
            shorthands.push(entry);
        } else {
            longhands.push(entry);
        }
    }

    longhands.sort_by(|a, b| a.name.cmp(&b.name));
    shorthands.sort_by(|a, b| a.name.cmp(&b.name));
    (longhands, shorthands)
}

/// The `PropertyId` expression naming `property`, for a shorthand's longhand list.
fn id_expression(name: &str, longhands: &[Entry], shorthands: &[Entry]) -> Option<String> {
    if let Some(entry) = longhands.iter().find(|entry| entry.name == name) {
        return Some(format!("PropertyId::Longhand(LonghandId::{})", entry.variant));
    }
    let entry = shorthands.iter().find(|entry| entry.name == name)?;
    Some(format!("PropertyId::Shorthand(ShorthandId::{})", entry.variant))
}

fn rust_string(text: &str) -> String {
    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            other => vec![other],
        })
        .collect();
    format!("\"{escaped}\"")
}

fn rust_option(text: Option<&String>) -> String {
    match text {
        Some(text) => format!("Some({})", rust_string(text)),
        None => "None".to_string(),
    }
}

/// Write the enum for one kind of property.
fn write_enum(out: &mut String, kind: &str, doc: &str, entries: &[Entry]) {
    let _ = writeln!(out, "{doc}");
    let _ = writeln!(out, "#[repr(u16)]");
    let _ = writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]"
    );
    let _ = writeln!(out, "pub enum {kind} {{");
    for (index, entry) in entries.iter().enumerate() {
        let _ = writeln!(out, "    /// `{}`", entry.name);
        let _ = writeln!(out, "    {} = {index},", entry.variant);
    }
    let _ = writeln!(out, "}}\n");
}

/// Everything the property set is, so a shorthand's longhand list can be written as ids.
struct Catalog<'a> {
    longhands: &'a [Entry],
    shorthands: &'a [Entry],
}

/// What one kind of property's table is called and what goes in it.
struct TableSpec {
    /// The enum the ids of this kind are.
    kind: &'static str,
    /// The struct one row of the table is.
    info: &'static str,
    /// The table's name.
    table: &'static str,
    /// The name of the `ALL_..` list of every id of this kind.
    all: &'static str,
    /// The constant holding how many there are.
    count: &'static str,
    /// Whether rows carry a longhand list, which only a shorthand has.
    with_longhands: bool,
}

/// Write the `[..Info; N]` table and the `ALL` list for one kind of property.
fn write_table(out: &mut String, spec: &TableSpec, entries: &[Entry], catalog: &Catalog<'_>) {
    let TableSpec {
        kind,
        info,
        table,
        all,
        count,
        with_longhands,
    } = *spec;
    let (longhands, shorthands) = (catalog.longhands, catalog.shorthands);
    // The tables are one line per property, which is far past the formatter's width and
    // exactly how they want to be read.
    let _ = writeln!(out, "#[rustfmt::skip]\nstatic {table}: [{info}; {count}] = [");
    for entry in entries {
        let _ = write!(
            out,
            "    {info} {{ name: {}, inherited: {}, initial: {}, percentage_is_number: {}",
            rust_string(&entry.name),
            entry.inherited,
            rust_option(entry.initial.as_ref()),
            entry.percentage_is_number,
        );
        if with_longhands {
            let ids: Vec<String> = entry
                .longhands
                .iter()
                .filter_map(|name| id_expression(name, longhands, shorthands))
                .collect();
            let _ = write!(out, ", longhands: &[{}]", ids.join(", "));
        }
        let _ = writeln!(out, " }},");
    }
    let _ = writeln!(out, "];\n");

    let _ = writeln!(
        out,
        "/// Every [`{kind}`], in id order - which is name order.\n#[rustfmt::skip]\npub static {all}: [{kind}; {count}] = ["
    );
    let mut line = String::from("   ");
    for entry in entries {
        let item = format!(" {kind}::{},", entry.variant);
        if line.len() + item.len() > 116 {
            let _ = writeln!(out, "{line}");
            line = String::from("   ");
        }
        line.push_str(&item);
    }
    if line.trim().is_empty() {
        let _ = writeln!(out, "];\n");
    } else {
        let _ = writeln!(out, "{line}\n];\n");
    }
}

/// Render the whole module.
fn render(longhands: &[Entry], shorthands: &[Entry]) -> String {
    let mut out = String::with_capacity(256 * 1024);

    out.push_str(
        "//! Property ids: one small integer per CSS property, generated from the property\n\
         //! definitions this crate embeds.\n\
         //!\n\
         //! The cascade is keyed by these rather than by property names. A name is a string to\n\
         //! hash, a string to allocate and a string to compare; an id is an array index, and a\n\
         //! dense one, so a property map is a slot per id rather than a hash table.\n\
         //!\n\
         //! Custom properties (`--*`) get no id. They are unbounded in number and cascade in a\n\
         //! pass of their own, so they stay in a map keyed by their full name.\n\
         //!\n\
         //! The tables here hold what can be read straight out of the definition data. Anything\n\
         //! that needs the resolved value grammar - whether a property takes a colour, the range\n\
         //! its computed value is clamped to - stays on [`crate::matcher::property_definitions::\
         PropertyDefinition`],\n\
         //! which is reachable by id through\n\
         //! [`CssDefinitions::definition`](crate::matcher::property_definitions::CssDefinitions::definition).\n\
         //!\n\
         //! GENERATED FILE - do not edit. Regenerate after changing the definition JSON with:\n\
         //!\n\
         //! ```text\n\
         //! cargo run -p generate_definitions -- --property-ids\n\
         //! ```\n\
         //!\n\
         //! That mode reads the checked-in `resources/definitions/definitions_properties.json`\n\
         //! and touches the network for nothing. `property_ids_match_the_definitions` in this\n\
         //! crate fails if the two ever drift apart.\n\n",
    );

    let _ = writeln!(
        out,
        "/// How many longhand properties there are.\npub const LONGHAND_COUNT: usize = {};\n",
        longhands.len()
    );
    let _ = writeln!(
        out,
        "/// How many shorthand properties there are.\npub const SHORTHAND_COUNT: usize = {};\n",
        shorthands.len()
    );
    out.push_str(
        "/// How many properties have an id, longhands and shorthands together. Every\n\
         /// [`PropertyId::index`] is below this, so an array of this length is a slot per\n\
         /// property.\n\
         pub const PROPERTY_COUNT: usize = LONGHAND_COUNT + SHORTHAND_COUNT;\n\n",
    );

    write_enum(
        &mut out,
        "LonghandId",
        "/// A property that is not a shorthand: it holds a value of its own.",
        longhands,
    );
    write_enum(
        &mut out,
        "ShorthandId",
        "/// A property whose value is distributed over other properties; see\n\
         /// [`ShorthandId::longhands`].",
        shorthands,
    );

    out.push_str(
        "/// A property this engine knows, as a longhand or a shorthand.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]\n\
         pub enum PropertyId {\n\
         \x20   Longhand(LonghandId),\n\
         \x20   Shorthand(ShorthandId),\n\
         }\n\n",
    );

    out.push_str(
        "/// What the definition data says about one longhand.\n\
         struct LonghandInfo {\n\
         \x20   name: &'static str,\n\
         \x20   inherited: bool,\n\
         \x20   /// The initial value as written in the definition data, before it is parsed.\n\
         \x20   initial: Option<&'static str>,\n\
         \x20   percentage_is_number: bool,\n\
         }\n\n\
         /// What the definition data says about one shorthand.\n\
         struct ShorthandInfo {\n\
         \x20   name: &'static str,\n\
         \x20   inherited: bool,\n\
         \x20   initial: Option<&'static str>,\n\
         \x20   percentage_is_number: bool,\n\
         \x20   /// The properties the shorthand sets, as its `computed` list names them. A few\n\
         \x20   /// of these are shorthands in their own right (`border` sets `border-color`).\n\
         \x20   longhands: &'static [PropertyId],\n\
         }\n\n",
    );

    let catalog = Catalog { longhands, shorthands };
    write_table(
        &mut out,
        &TableSpec {
            kind: "LonghandId",
            info: "LonghandInfo",
            table: "LONGHANDS",
            all: "ALL_LONGHAND_IDS",
            count: "LONGHAND_COUNT",
            with_longhands: false,
        },
        longhands,
        &catalog,
    );
    write_table(
        &mut out,
        &TableSpec {
            kind: "ShorthandId",
            info: "ShorthandInfo",
            table: "SHORTHANDS",
            all: "ALL_SHORTHAND_IDS",
            count: "SHORTHAND_COUNT",
            with_longhands: true,
        },
        shorthands,
        &catalog,
    );

    // Every property by name, sorted, for the binary search in `PropertyId::from_name`.
    let mut by_name: Vec<(&str, String)> = Vec::with_capacity(longhands.len() + shorthands.len());
    for entry in longhands {
        by_name.push((
            entry.name.as_str(),
            format!("PropertyId::Longhand(LonghandId::{})", entry.variant),
        ));
    }
    for entry in shorthands {
        by_name.push((
            entry.name.as_str(),
            format!("PropertyId::Shorthand(ShorthandId::{})", entry.variant),
        ));
    }
    by_name.sort_by(|a, b| a.0.cmp(b.0));

    out.push_str(
        "/// Every property name with its id, sorted by name so [`PropertyId::from_name`] can\n\
         /// binary-search it.\n",
    );
    let _ = writeln!(
        out,
        "#[rustfmt::skip]\nstatic BY_NAME: [(&str, PropertyId); PROPERTY_COUNT] = ["
    );
    for (name, id) in &by_name {
        let _ = writeln!(out, "    ({}, {id}),", rust_string(name));
    }
    let _ = writeln!(out, "];\n");

    out.push_str(include_str!("property_ids_impl.rs"));

    out
}

/// Read `input` and write the property-id module to `output`.
pub fn generate(input: &Path, output: &Path) -> Result<()> {
    let text = fs::read_to_string(input).with_context(|| format!("reading {}", input.display()))?;
    let properties: Vec<Property> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", input.display()))?;

    let (longhands, shorthands) = build_entries(&properties);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, render(&longhands, &shorthands))?;

    eprintln!(
        "wrote {} ({} longhands, {} shorthands)",
        output.display(),
        longhands.len(),
        shorthands.len()
    );
    Ok(())
}
