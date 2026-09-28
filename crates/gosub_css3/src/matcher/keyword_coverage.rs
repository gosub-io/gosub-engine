//! Does every keyword the grammar accepts mean something in the typed style?
//!
//! The typed style maps each keyword property by hand (`computed_style.rs`), against a grammar
//! that comes from the definitions data. Nothing ties the two together, and they drift: a keyword
//! the grammar accepts can land on a match's `_ =>` fallback and quietly become some other value,
//! or the mapping can drop it and the property is never set. `text-align: -webkit-center` was an
//! arm the grammar never let through; `float: inline-start` validates and then does nothing.
//!
//! This walks every keyword each engine-read property's grammar accepts on its own, computes a
//! style from it, and reports two things: keywords the typed style drops, and keywords that land
//! on the same typed value as a different keyword. The known cases are listed in [`KNOWN_GAPS`],
//! so the test fails on any new drift, and on a listed gap that has gone away - the list is the
//! report of what is still wrong, and has to stay true.

#![cfg(test)]

use std::collections::BTreeMap;

use cow_utils::CowUtils;
use std::sync::Arc;

use gosub_interface::css3::CssOrigin;
use gosub_interface::style::{ComputedStyle, Prop};
use gosub_shared::config::ParserConfig;

use crate::matcher::computed_style::computed_style;
use crate::matcher::expansion::ExpandedDeclaration;
use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::styling::{no_location, CssProperties, CssProperty, DeclarationProperty};
use crate::matcher::syntax::SyntaxComponent;
use crate::stylesheet::Specificity;
use crate::Css3;

/// The keyword properties the engine reads, with the typed field each one lands in.
type Field = fn(&ComputedStyle) -> String;

const PROPERTIES: &[(&str, Prop, Field)] = &[
    ("display", Prop::Display, |s| format!("{:?}", s.box_group.display)),
    ("position", Prop::Position, |s| format!("{:?}", s.box_group.position)),
    ("float", Prop::Float, |s| format!("{:?}", s.box_group.float)),
    ("clear", Prop::Clear, |s| format!("{:?}", s.box_group.clear)),
    ("box-sizing", Prop::BoxSizing, |s| {
        format!("{:?}", s.box_group.box_sizing)
    }),
    ("overflow-x", Prop::OverflowX, |s| {
        format!("{:?}", s.box_group.overflow_x)
    }),
    ("overflow-y", Prop::OverflowY, |s| {
        format!("{:?}", s.box_group.overflow_y)
    }),
    ("scrollbar-width", Prop::ScrollbarWidth, |s| {
        format!("{:?}", s.box_group.scrollbar_width)
    }),
    ("table-layout", Prop::TableLayout, |s| {
        format!("{:?}", s.box_group.table_layout)
    }),
    ("vertical-align", Prop::VerticalAlign, |s| {
        format!("{:?}", s.box_group.vertical_align)
    }),
    ("text-align", Prop::TextAlign, |s| {
        format!("{:?}", s.inherited.text_align)
    }),
    ("text-transform", Prop::TextTransform, |s| {
        format!("{:?}", s.inherited.text_transform)
    }),
    ("font-style", Prop::FontStyle, |s| {
        format!("{:?}", s.inherited.font_style)
    }),
    ("font-weight", Prop::FontWeight, |s| {
        format!("{:?}", s.inherited.font_weight)
    }),
    ("white-space", Prop::WhiteSpace, |s| {
        format!("{:?}", s.inherited.white_space)
    }),
    ("text-wrap", Prop::TextWrap, |s| format!("{:?}", s.box_group.text_wrap)),
    ("caption-side", Prop::CaptionSide, |s| {
        format!("{:?}", s.inherited.caption_side)
    }),
    ("border-collapse", Prop::BorderCollapse, |s| {
        format!("{:?}", s.inherited.border_collapse)
    }),
    ("border-top-style", Prop::BorderTopStyle, |s| {
        format!("{:?}", s.border.top_style)
    }),
    ("outline-style", Prop::OutlineStyle, |s| {
        format!("{:?}", s.outline.style)
    }),
    ("flex-direction", Prop::FlexDirection, |s| {
        format!("{:?}", s.flex.direction)
    }),
    ("flex-wrap", Prop::FlexWrap, |s| format!("{:?}", s.flex.wrap)),
    ("align-items", Prop::AlignItems, |s| format!("{:?}", s.flex.align_items)),
    ("align-self", Prop::AlignSelf, |s| format!("{:?}", s.flex.align_self)),
    ("align-content", Prop::AlignContent, |s| {
        format!("{:?}", s.flex.align_content)
    }),
    ("justify-items", Prop::JustifyItems, |s| {
        format!("{:?}", s.flex.justify_items)
    }),
    ("justify-self", Prop::JustifySelf, |s| {
        format!("{:?}", s.flex.justify_self)
    }),
    ("justify-content", Prop::JustifyContent, |s| {
        format!("{:?}", s.flex.justify_content)
    }),
    ("grid-auto-flow", Prop::GridAutoFlow, |s| {
        format!("{:?}", s.grid.auto_flow)
    }),
];

/// What is wrong with one keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Gap {
    /// Valid, and the typed style never records it.
    Dropped,
    /// Valid, and it lands on a typed value named for another keyword of the property - which is
    /// what a match's `_ =>` fallback does.
    Merged,
}

/// The gaps as they stand. Each is a keyword that validates and then means the wrong thing, or
/// nothing. Fixing one means taking it off this list; the test says so when that happens.
const KNOWN_GAPS: &[(&str, &str, Gap)] = &[
    // align-items: the self-relative alignment keywords have no AlignValue of their own.
    ("align-items", "self-end", Gap::Merged),
    ("align-items", "self-start", Gap::Merged),
    // align-self: the self-relative and anchor alignment keywords have no AlignValue of their own.
    ("align-self", "anchor-center", Gap::Merged),
    ("align-self", "self-end", Gap::Merged),
    ("align-self", "self-start", Gap::Merged),
    // clear: logical, page-float and both-axis values: no Clear variant, so they clear nothing.
    ("clear", "block-end", Gap::Merged),
    ("clear", "block-start", Gap::Merged),
    ("clear", "both-block", Gap::Merged),
    ("clear", "both-inline", Gap::Merged),
    ("clear", "bottom", Gap::Merged),
    ("clear", "inline-end", Gap::Merged),
    ("clear", "inline-start", Gap::Merged),
    ("clear", "top", Gap::Merged),
    // display: no Display variant, so each becomes a block (css-display-3 gives each its own box).
    ("display", "contents", Gap::Merged),
    ("display", "flow-root", Gap::Merged),
    ("display", "list-item", Gap::Merged),
    ("display", "math", Gap::Merged),
    ("display", "ruby", Gap::Merged),
    ("display", "ruby-base", Gap::Merged),
    ("display", "ruby-base-container", Gap::Merged),
    ("display", "ruby-text", Gap::Merged),
    ("display", "ruby-text-container", Gap::Merged),
    ("display", "run-in", Gap::Merged),
    // flex-wrap: `balance` (css-flexbox-2) wraps like `wrap`, not `nowrap`.
    ("flex-wrap", "balance", Gap::Merged),
    // float: logical and page floats: no Float variant, so they do not float.
    ("float", "block-end", Gap::Merged),
    ("float", "block-start", Gap::Merged),
    ("float", "bottom", Gap::Merged),
    ("float", "footnote", Gap::Merged),
    ("float", "inline-end", Gap::Merged),
    ("float", "inline-start", Gap::Merged),
    ("float", "snap-block", Gap::Merged),
    ("float", "snap-inline", Gap::Merged),
    ("float", "top", Gap::Merged),
    // font-style: `left`/`right` (the oblique direction, css-fonts-5) become normal.
    ("font-style", "left", Gap::Merged),
    ("font-style", "right", Gap::Merged),
    // grid-auto-flow: `dense` alone is `row dense`; the packing mode is lost.
    ("grid-auto-flow", "dense", Gap::Merged),
    // justify-content: `left`/`right` have no AlignValue.
    ("justify-content", "left", Gap::Merged),
    ("justify-content", "right", Gap::Merged),
    // justify-items: the self-relative, anchor and left/right keywords have no AlignValue of their own.
    ("justify-items", "left", Gap::Merged),
    ("justify-items", "right", Gap::Merged),
    ("justify-items", "self-end", Gap::Merged),
    ("justify-items", "self-start", Gap::Merged),
    // justify-self: the self-relative, anchor and left/right keywords have no AlignValue of their own.
    ("justify-self", "anchor-center", Gap::Merged),
    ("justify-self", "left", Gap::Merged),
    ("justify-self", "right", Gap::Merged),
    ("justify-self", "self-end", Gap::Merged),
    ("justify-self", "self-start", Gap::Merged),
    // outline-style: `auto` is its own value (css-ui-4), the UA's focus ring; it becomes `solid`.
    ("outline-style", "auto", Gap::Merged),
    // scrollbar-width: only a plain number is read, so none of its keywords is ever set.
    ("scrollbar-width", "auto", Gap::Dropped),
    ("scrollbar-width", "none", Gap::Dropped),
    ("scrollbar-width", "thin", Gap::Dropped),
    // text-align: `justify-all` justifies the last line too; it becomes `left`.
    ("text-align", "justify-all", Gap::Merged),
    // text-transform: `full-width`/`full-size-kana` do nothing.
    ("text-transform", "full-size-kana", Gap::Merged),
    ("text-transform", "full-width", Gap::Merged),
    // text-wrap: `auto` has no TextWrap variant and lands on `wrap`.
    ("text-wrap", "auto", Gap::Merged),
    // vertical-align: the baseline keywords of css-inline-3 land on `Baseline` or `Other`.
    ("vertical-align", "alphabetic", Gap::Merged),
    ("vertical-align", "center", Gap::Merged),
    ("vertical-align", "central", Gap::Merged),
    ("vertical-align", "first", Gap::Merged),
    ("vertical-align", "hanging", Gap::Merged),
    ("vertical-align", "ideographic", Gap::Merged),
    ("vertical-align", "last", Gap::Merged),
    ("vertical-align", "mathematical", Gap::Merged),
];

/// Keywords that share a typed value with another keyword because the spec says they mean the
/// same thing. These are not gaps.
const ALIASES: &[(&str, &str, Gap)] = &[
    // `display: flow` is the block-level `display: block flow`, which is `block` (css-display-3 §2).
    ("display", "flow", Gap::Merged),
];

/// The single keywords `name`'s grammar accepts, whether written in the property's own grammar
/// or in a type it references.
fn grammar_keywords(name: &str) -> Vec<String> {
    fn walk(component: &SyntaxComponent, out: &mut Vec<String>) {
        match component {
            SyntaxComponent::GenericKeyword { keyword, .. } => out.push(keyword.cow_to_ascii_lowercase().into_owned()),
            SyntaxComponent::Group { components, .. } => components.iter().for_each(|c| walk(c, out)),
            SyntaxComponent::Function { .. } => {}
            _ => {}
        }
    }
    let definitions = get_css_definitions();
    let definition = definitions
        .find_property(name)
        .expect("an engine property has a definition");
    let mut out = Vec::new();
    definition.syntax().components.iter().for_each(|c| walk(c, &mut out));
    out.sort();
    out.dedup();
    out
}

/// The style `name: value` computes to on an element with nothing else declared, or `None` when
/// the declaration is not valid on its own.
fn style_of(name: &str, value: &str) -> Option<ComputedStyle> {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    let sheet = Css3::parse_str(
        &format!("a {{ {name}: {value} }}"),
        config,
        CssOrigin::Author,
        "coverage",
    )
    .ok()?;
    let rule = sheet.rules.first()?;
    let mut map = CssProperties::new();
    let mut valid = false;
    for expanded in rule.expanded() {
        let ExpandedDeclaration::Resolved { entries, important } = expanded else {
            continue;
        };
        valid = true;
        for (order, (id, value)) in entries.iter().enumerate() {
            if map.get_id(*id).is_none() {
                map.insert_id(*id, CssProperty::new(*id));
            }
            let property = map.get_id_mut(*id).expect("just inserted");
            property.declared.push(DeclarationProperty {
                value: Arc::clone(value),
                origin: CssOrigin::Author,
                important: *important,
                location: no_location(),
                specificity: Specificity::new(0, 0, 1),
                shadow_depth: 0,
                order: u32::try_from(order + 1).expect("a handful of entries"),
                layer: None,
                attached: false,
            });
        }
    }
    if !valid {
        return None;
    }
    for (_, property) in map.iter_ids_mut() {
        property.font_size_basis = 16.0;
        property.root_font_size_basis = 16.0;
        property.mark_dirty();
        property.compute_value();
    }
    Some(computed_style(&map, None))
}

/// Every gap in the typed style's reading of the engine properties, in a stable order.
fn gaps() -> Vec<(String, String, Gap)> {
    let mut found = Vec::new();
    for (name, prop, field) in PROPERTIES {
        let mut by_value: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for keyword in grammar_keywords(name) {
            let Some(style) = style_of(name, &keyword) else {
                continue;
            };
            if style.has(*prop) {
                by_value.entry(field(&style)).or_default().push(keyword);
            } else {
                found.push(((*name).to_string(), keyword, Gap::Dropped));
            }
        }
        // Several keywords on one value: the keyword the value is named for owns it (`none` for
        // `None`), and the others fell onto it. With no owner, all of them did.
        for (value, keywords) in by_value.iter().filter(|(_, keywords)| keywords.len() > 1) {
            let named = |keyword: &str| keyword.cow_replace('-', "").eq_ignore_ascii_case(value);
            let has_owner = keywords.iter().any(|keyword| named(keyword));
            for keyword in keywords.iter().filter(|keyword| !(has_owner && named(keyword))) {
                found.push(((*name).to_string(), keyword.clone(), Gap::Merged));
            }
        }
    }
    found.retain(|(name, keyword, gap)| !ALIASES.contains(&(name.as_str(), keyword.as_str(), *gap)));
    found.sort();
    found
}

#[test]
fn every_grammar_keyword_means_something() {
    let found = gaps();
    let known: Vec<(String, String, Gap)> = KNOWN_GAPS
        .iter()
        .map(|(name, keyword, gap)| ((*name).to_string(), (*keyword).to_string(), *gap))
        .collect();

    let new: Vec<_> = found.iter().filter(|gap| !known.contains(gap)).collect();
    let fixed: Vec<_> = known.iter().filter(|gap| !found.contains(gap)).collect();
    let report = |gaps: &[&(String, String, Gap)]| {
        gaps.iter()
            .map(|(name, keyword, gap)| format!("    (\"{name}\", \"{keyword}\", Gap::{gap:?}),"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        new.is_empty() && fixed.is_empty(),
        "keyword coverage changed.\nNew gaps - map the keyword, or list it in KNOWN_GAPS:\n{}\nGone - take them off KNOWN_GAPS:\n{}",
        report(&new),
        report(&fixed),
    );
}
