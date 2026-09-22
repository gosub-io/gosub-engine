//! What styling costs an element, with the sharing made visible.
//!
//! The style system's memory story is that elements share. A `ComputedStyle` is eleven `Arc`s of
//! which most point at the parent's groups or at one process-wide initial group; the inheritance
//! chain is one record per element rather than a copy per child; ancestor filters and
//! custom-property scopes are handed down too. A report that walked every `Arc` from every
//! element would price all of that per element, which is exactly the number the rebuild set out
//! to make false. So the walk counts each shared allocation once and keeps those bytes in their
//! own column - see [`gosub_shared::memory`].
//!
//! The `HeapSize` implementations live beside their types, where the private fields are; this is
//! only the collector that turns them into report rows.

use gosub_shared::memory::{record, HeapSize, Row, Walk};

use crate::matcher::expansion::ExpandedDeclaration;
use crate::matcher::styling::{CssProperties, CssProperty};
use crate::stylesheet::{CssRule, CssStylesheet};

/// Add a row per part of every element's property map to the current snapshot.
///
/// Four rows, because the four parts behave differently as a page grows: what an element
/// actually cascaded, the slot table it pays for whether it declared one property or fifty, and
/// the two things it mostly shares - the inheritance chain and the scopes handed down to it.
///
/// `walk` is passed in rather than made here so the whole snapshot shares one set of
/// already-counted allocations: a stylesheet reached from a declaration is not counted again
/// when the stylesheet row reaches it.
pub fn record_property_maps<'a, I>(maps: impl Fn() -> I, walk: &mut Walk)
where
    I: Iterator<Item = &'a CssProperties>,
{
    let mut elements = 0u64;
    let mut declared = 0u64;
    let mut declared_entries = 0u64;

    // The map itself: the struct per element and the property slots it holds, without the
    // values hanging off them - those are the two rows below, so nothing is counted twice.
    for map in maps() {
        elements += 1;
        declared += map.props_slice().len() as u64;
        walk.bytes(map.props_capacity() * size_of::<CssProperty>());
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new(
            "css.property_maps",
            declared,
            elements as usize * size_of::<CssProperties>(),
            owned,
            shared,
        )
        .with_note(format!(
            "{elements} maps, {:.1} declared properties each",
            if elements == 0 {
                0.0
            } else {
                declared as f64 / elements as f64
            }
        )),
    );

    // Every declaration that reached a property, winner and losers alike - the losers are kept
    // because `revert` asks what the cascade would have said without an origin. This is where
    // the sheet's values end up, one clone per element that matched.
    for map in maps() {
        for property in map.props_slice() {
            declared_entries += property.declared.len() as u64;
            property.declared.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("css.declared_entries", declared_entries, 0, owned, shared)
            .with_note("one per declaration that reached a property, cloned from the rule"),
    );

    for map in maps() {
        for property in map.props_slice() {
            property.computed.heap_size(walk);
            property.inherited.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("css.settled_values", declared, 0, owned, shared)
            .with_note("the computed and inherited value each property settled on"),
    );

    let mut slots = 0usize;
    for map in maps() {
        slots += map.slot_len();
    }
    walk.bytes(slots * size_of::<u16>());
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("css.slot_tables", elements, 0, owned, shared)
            .with_note("one u16 per known property, on every element, declared or not"),
    );

    for map in maps() {
        if let Some(chain) = map.inherited_record() {
            chain.heap_size(walk);
        }
        if let Some(chain) = map.handed_down_record() {
            chain.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("css.inheritance_chain", elements, 0, owned, shared)
            .with_note("one record per element, shared by every child; a leaf builds none"),
    );

    for map in maps() {
        map.custom.heap_size(walk);
        if let Some(filter) = map.ancestor_filter() {
            filter.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("css.scopes_and_filters", elements, 0, owned, shared)
            .with_note("custom-property scopes and ancestor bloom filters, both handed down"),
    );
}

/// Add rows for the parsed stylesheets to the current snapshot.
///
/// Four rows, because they answer different questions: how much the rules themselves cost, how
/// much their selectors cost, how much their declarations cost, and how much the per-rule
/// expansion cache has grown since the page was styled.
pub fn record_stylesheets<'a, I>(sheets: impl Fn() -> I, walk: &mut Walk)
where
    I: Iterator<Item = &'a CssStylesheet>,
{
    let mut rules = 0u64;
    let mut selectors = 0u64;
    let mut declarations = 0u64;
    let mut expanded_rules = 0u64;

    for sheet in sheets() {
        rules += sheet.rules.len() as u64;
        walk.bytes(sheet.rules.capacity() * size_of::<CssRule>());
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("sheet.rules", rules, 0, owned, shared)
            .with_note("the rule structs themselves, before their contents"),
    );

    for sheet in sheets() {
        for rule in &sheet.rules {
            selectors += rule.selectors.len() as u64;
            rule.selectors.heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("sheet.selectors", selectors, 0, owned, shared)
            .with_note("parts, specificities and the precomputed ancestor keys"),
    );

    for sheet in sheets() {
        for rule in &sheet.rules {
            declarations += rule.declarations().len() as u64;
            rule.declarations().heap_size(walk);
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("sheet.declarations", declarations, 0, owned, shared)
            .with_note("property name and parsed value, as written"),
    );

    for sheet in sheets() {
        for rule in &sheet.rules {
            let Some(expanded) = rule.expanded_if_built() else {
                continue;
            };
            expanded_rules += 1;
            walk.bytes(expanded.capacity() * size_of::<ExpandedDeclaration>());
            for declaration in expanded {
                declaration.heap_size(walk);
            }
        }
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("sheet.expansion_cache", expanded_rules, 0, owned, shared)
            .with_note("validated and shorthand-expanded declarations, built per rule on first use"),
    );
}

/// Add a row for the selector index each stylesheet builds.
///
/// Built lazily on the first element styled against the sheet, and it lives as long as the
/// sheet does, so on a page that styles anything it is as real as the rules themselves.
pub fn record_selector_index<'a, I>(sheets: impl Fn() -> I, walk: &mut Walk)
where
    I: Iterator<Item = &'a CssStylesheet>,
{
    let mut indexed = 0u64;
    for sheet in sheets() {
        let index = sheet.index.read();
        let Some(index) = index.as_ref() else {
            continue;
        };
        indexed += 1;
        index.heap_size(walk);
    }
    let (owned, shared) = walk.take_counts();
    record(
        Row::new("sheet.selector_index", indexed, 0, owned, shared)
            .with_note("rules filed by their rightmost compound: id, class, tag, attribute, universal"),
    );
}
