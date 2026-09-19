//! Validating and expanding a rule's declarations once, instead of once per element.
//!
//! Whether a declaration is valid, and which longhands a shorthand expands to, depends on the
//! declaration alone - not on the element it is being applied to. A rule that matches a
//! thousand elements used to run the grammar matcher and the shorthand resolver a thousand
//! times over the same text to reach the same answer every time.
//!
//! So the work is done once and kept on the rule ([`crate::stylesheet::CssRule::expanded`]),
//! and the cascade is left with what genuinely differs per element: which rules matched, and
//! at what specificity, order and depth their declarations enter the map.
//!
//! The exception is a value carrying a substitution function. `var()` reads the custom
//! properties in scope on the element, `attr()` reads the element's attributes and
//! `light-dark()` reads the colour scheme, so none of the three can be resolved - and
//! therefore none can be validated or expanded - before the element is known. Those stay on
//! the per-element path.

use crate::matcher::property_definitions::get_css_definitions;
use crate::matcher::shorthands::{FixList, FixListInfo};
use crate::stylesheet::{CssDeclaration, CssValue, Specificity};
use gosub_interface::css3::CssOrigin;

/// The functions whose result depends on the element or the environment rather than on the
/// declaration; see [`crate::system::resolve_functions`].
const SUBSTITUTION_FUNCTIONS: [&str; 4] = ["var", "attr", "light-dark", "-internal-light-dark"];

/// One source declaration, prepared as far as it can be without an element.
#[derive(Debug, Clone, PartialEq)]
pub enum ExpandedDeclaration {
    /// A custom property (`--x`). Custom properties cascade in a pass of their own, before the
    /// regular one, so this carries nothing.
    Custom,
    /// A property this engine has no definition for, or a value its grammar rejects. Either way
    /// the declaration is invalid (css-syntax-3 §9) and contributes nothing to any element.
    Invalid,
    /// The value holds a substitution function, so the element decides what it says. Validation
    /// and expansion happen per element, after the function is resolved.
    Pending,
    /// Ready to cascade: the declaration under its own name, followed by every longhand the
    /// shorthand expansion produced.
    Resolved {
        /// `(property, value)` in the order they enter the element's map.
        entries: Vec<(String, CssValue)>,
        /// The declaration's `!important` flag, carried by every entry above.
        important: bool,
    },
}

/// Prepare every declaration of a rule. Called once per rule, the first time an element makes
/// the rule matter.
#[must_use]
pub fn expand_declarations(declarations: &[CssDeclaration]) -> Vec<ExpandedDeclaration> {
    declarations.iter().map(expand_declaration).collect()
}

fn expand_declaration(declaration: &CssDeclaration) -> ExpandedDeclaration {
    if declaration.property.starts_with("--") {
        return ExpandedDeclaration::Custom;
    }
    if needs_element(&declaration.value) {
        return ExpandedDeclaration::Pending;
    }

    let definitions = get_css_definitions();
    let Some(definition) = definitions.find_property(&declaration.property) else {
        // A property this engine has no definition for is a property it does not support, and a
        // declaration for one is invalid (css-syntax-3 §9). It is dropped rather than passed
        // through: an unvalidated value reaching the style consumer is how `dsiplay: block` used
        // to be recorded and answered by `getComputedStyle` as though it were a real declaration.
        //
        // What reaches this is misspellings, properties from specs the definitions data does not
        // cover, and the few `-internal-` names the user-agent sheet sets and nothing reads.
        log::debug!("Unknown property, declaration dropped: {}", declaration.property);
        return ExpandedDeclaration::Invalid;
    };

    let input = declaration.value.to_slice();

    // The longhands are expanded into a list of their own, which is why this can be done without
    // an element at all: the cascade facts are stamped on when the element is known.
    let mut fix_list = FixList::new();
    fix_list.set_info(placeholder_info());
    fix_list.reset_multiplier(&declaration.property);
    if !definition.matches_and_shorthands(input, &mut fix_list) {
        log::debug!("Declaration does not match definition: {declaration:?}");
        return ExpandedDeclaration::Invalid;
    }
    // A shorthand sets every one of its longhands; the ones it left out are reset to their
    // initial value.
    fix_list.reset_unmentioned(definition, input, definitions);
    // A longhand that is itself a shorthand (`border-color` under `border`) is expanded in turn.
    fix_list.resolve_nested(definitions);

    // The declaration itself is kept under its own name as well as expanded: the render pipeline
    // reads shorthand keys (`background`, `padding`, `text-decoration`) directly.
    let mut entries = Vec::with_capacity(fix_list.entry_count() + 1);
    entries.push((declaration.property.clone(), single_value(declaration.value.clone())));
    entries.extend(fix_list.into_entries());

    ExpandedDeclaration::Resolved {
        entries,
        important: declaration.important,
    }
}

/// A one-element list is the value itself. `resolve_functions` wraps what it resolves in a list,
/// and the per-element path has always unwrapped it again before storing the declaration; a
/// declaration that never went through a function has to arrive at the same shape.
#[must_use]
pub fn single_value(value: CssValue) -> CssValue {
    let CssValue::List(mut values) = value else {
        return value;
    };
    match values.pop() {
        Some(single) if values.is_empty() => single,
        Some(last) => {
            values.push(last);
            CssValue::List(values)
        }
        None => CssValue::List(values),
    }
}

/// The cascade facts of a declaration being expanded without an element in hand.
///
/// Nothing reads them: the expansion records values, and the element's own origin, specificity
/// and order are stamped on when the values reach its map. They matter only inside the
/// expansion, where every entry carries the same ones and so compares equal to every other -
/// which is exactly what the real expansion of a single declaration does too, and what makes
/// the last write for a longhand win.
fn placeholder_info() -> FixListInfo {
    FixListInfo::new(
        CssOrigin::Author,
        false,
        String::new(),
        Specificity::new(0, 0, 0),
        0,
        0,
        None,
        false,
    )
}

/// Whether resolving this value would ask the element or the environment anything.
///
/// The scan goes as deep as the value nests, which is deeper than `resolve_functions` itself
/// looks: a value it would not resolve costs a little caching and nothing else, while one it
/// would resolve and this missed would be cached with the wrong answer.
fn needs_element(value: &CssValue) -> bool {
    match value {
        CssValue::Function(name, args) => {
            SUBSTITUTION_FUNCTIONS.iter().any(|f| name.eq_ignore_ascii_case(f)) || args.iter().any(needs_element)
        }
        CssValue::List(values) => values.iter().any(needs_element),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(property: &str, value: CssValue) -> CssDeclaration {
        CssDeclaration {
            property: property.to_string(),
            value,
            important: false,
        }
    }

    fn entries(declaration: &CssDeclaration) -> Vec<(String, CssValue)> {
        match expand_declaration(declaration) {
            ExpandedDeclaration::Resolved { entries, .. } => entries,
            other => panic!("expected a resolved declaration, got {other:?}"),
        }
    }

    #[test]
    fn unknown_property_is_invalid() {
        let declaration = declaration("dsiplay", CssValue::String("block".to_string()));
        assert_eq!(expand_declaration(&declaration), ExpandedDeclaration::Invalid);
    }

    #[test]
    fn value_the_grammar_rejects_is_invalid() {
        let declaration = declaration("display", CssValue::Unit(10.0, "px".to_string()));
        assert_eq!(expand_declaration(&declaration), ExpandedDeclaration::Invalid);
    }

    #[test]
    fn custom_property_is_left_to_its_own_pass() {
        let declaration = declaration("--brand", CssValue::String("red".to_string()));
        assert_eq!(expand_declaration(&declaration), ExpandedDeclaration::Custom);
    }

    #[test]
    fn substitution_function_waits_for_the_element() {
        let var = CssValue::Function("var".to_string(), vec![CssValue::String("--brand".to_string())]);
        assert_eq!(
            expand_declaration(&declaration("color", var)),
            ExpandedDeclaration::Pending
        );

        // Nested as deep as the value goes, not only at the top.
        let nested = CssValue::List(vec![
            CssValue::Unit(1.0, "px".to_string()),
            CssValue::Function(
                "rgb".to_string(),
                vec![CssValue::Function(
                    "var".to_string(),
                    vec![CssValue::String("--c".to_string())],
                )],
            ),
        ]);
        assert_eq!(
            expand_declaration(&declaration("border", nested)),
            ExpandedDeclaration::Pending
        );
    }

    #[test]
    fn a_longhand_expands_to_itself_only() {
        let declaration = declaration("color", CssValue::String("red".to_string()));
        assert_eq!(
            entries(&declaration),
            vec![("color".to_string(), CssValue::String("red".to_string()))]
        );
    }

    #[test]
    fn a_shorthand_keeps_its_own_name_and_gains_its_longhands() {
        let declaration = declaration(
            "margin",
            CssValue::List(vec![
                CssValue::Unit(1.0, "px".to_string()),
                CssValue::Unit(2.0, "px".to_string()),
            ]),
        );
        let entries = entries(&declaration);
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names[0], "margin");
        for side in ["margin-top", "margin-right", "margin-bottom", "margin-left"] {
            assert!(names.contains(&side), "{side} missing from {names:?}");
        }
        let value = |name: &str| {
            entries
                .iter()
                .find(|(entry, _)| entry == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(value("margin-top"), Some(CssValue::Unit(1.0, "px".to_string())));
        assert_eq!(value("margin-left"), Some(CssValue::Unit(2.0, "px".to_string())));
    }

    #[test]
    fn a_single_element_list_is_unwrapped_under_its_own_name() {
        let declaration = declaration("color", CssValue::List(vec![CssValue::String("red".to_string())]));
        assert_eq!(entries(&declaration)[0].1, CssValue::String("red".to_string()));
    }
}
