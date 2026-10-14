//! Custom properties registered with `@property` (css-properties-values-api-1 §3).
//!
//! An unregistered custom property inherits and starts out as the guaranteed-invalid value, so
//! a `var()` of one nobody declared takes its fallback. A registered one starts out as its
//! `initial-value` everywhere, which a `var()` substitutes, and may not inherit at all. Tailwind
//! v4 leans on both: it registers its `--tw-*` properties with `inherits: false` and an initial
//! value, and its utilities read them through `var()` without a fallback.

use crate::media_query::media_environment;
use crate::stylesheet::{CssStylesheet, CssValue};
use std::collections::HashMap;

/// What one `@property` rule says about its property.
#[derive(Debug, PartialEq)]
pub struct Registration {
    pub inherits: bool,
    /// `None` only for the universal syntax with no `initial-value`, which keeps the
    /// guaranteed-invalid value as its initial value.
    pub initial: Option<CssValue>,
}

/// Every registration in effect for a document, merged across its sheets.
#[derive(Debug, PartialEq)]
pub struct PropertyRegistry {
    entries: HashMap<String, Registration>,
}

impl PropertyRegistry {
    /// The registrations in `sheets`, or `None` when there are none, which is every page that
    /// does not use `@property`. The last valid rule for a name wins, in sheet order and then
    /// source order. A rule under an `@media` that does not hold registers nothing, and a sheet
    /// inside a shadow tree registers nothing either: registration is document-wide, and the
    /// spec ignores `@property` in shadow trees.
    #[must_use]
    pub fn build(sheets: &[CssStylesheet]) -> Option<Self> {
        if sheets.iter().all(|sheet| sheet.property_rules.is_empty()) {
            return None;
        }
        let env = media_environment();
        let mut entries = HashMap::new();
        for sheet in sheets.iter().filter(|sheet| sheet.scope.is_none()) {
            for rule in &sheet.property_rules {
                let applies = rule
                    .media
                    .as_ref()
                    .is_none_or(|lists| lists.iter().all(|list| list.matches(&env)));
                if applies {
                    entries.insert(
                        rule.name.clone(),
                        Registration {
                            inherits: rule.inherits,
                            initial: rule.initial.clone(),
                        },
                    );
                }
            }
        }
        (!entries.is_empty()).then_some(Self { entries })
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Registration> {
        self.entries.get(name)
    }
}
