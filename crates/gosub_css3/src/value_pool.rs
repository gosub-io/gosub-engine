//! One allocation per distinct declared value, however many rules write it.
//!
//! A real-world stylesheet says the same thing over and over: on the 2.2 MB sheet the engine is
//! tested against, 37,706 declared values are only 4,689 distinct ones - `Zero` alone is written
//! 2,807 times. Each used to be its own allocation, and since a value is shared with every
//! element its rule matches (see [`crate::stylesheet::CssDeclaration::value`]), the duplication
//! was paid once per rule rather than once per page.
//!
//! The pool is filled while a sheet is parsed and dropped when the parse ends; what it handed
//! out stays alive in the sheet.
//!
//! # Identity, not equality
//!
//! The key is the value's exact representation. [`CssValue`]'s own `PartialEq` is not usable
//! here: for colours it asks whether two values *are the same colour*, comparing converted sRGB
//! with a tolerance, so `rgb(0 0 0)` and a colour a thousandth of a channel away compare equal.
//! Pooling on that would silently replace one with the other, and the survivor is what gets
//! serialised back to the page.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::stylesheet::CssValue;

/// A declared value keyed by what it is, exactly.
struct ExactValue(Arc<CssValue>);

impl PartialEq for ExactValue {
    fn eq(&self, other: &Self) -> bool {
        exactly_equal(&self.0, &other.0)
    }
}

impl Eq for ExactValue {}

impl Hash for ExactValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_value(&self.0, state);
    }
}

fn exactly_equal(a: &CssValue, b: &CssValue) -> bool {
    match (a, b) {
        (CssValue::None, CssValue::None)
        | (CssValue::Zero, CssValue::Zero)
        | (CssValue::Initial, CssValue::Initial)
        | (CssValue::Inherit, CssValue::Inherit)
        | (CssValue::Comma, CssValue::Comma) => true,
        (CssValue::Color(a), CssValue::Color(b)) => a.exact_parts() == b.exact_parts(),
        (CssValue::Number(a, ak), CssValue::Number(b, bk)) => a.to_bits() == b.to_bits() && ak == bk,
        (CssValue::Percentage(a), CssValue::Percentage(b)) => a.to_bits() == b.to_bits(),
        (CssValue::String(a), CssValue::String(b)) => a == b,
        (CssValue::Unit(a, au), CssValue::Unit(b, bu)) => a.to_bits() == b.to_bits() && au == bu,
        (CssValue::Function(an, aa), CssValue::Function(bn, ba)) => {
            an == bn && aa.len() == ba.len() && aa.iter().zip(ba).all(|(a, b)| exactly_equal(a, b))
        }
        (CssValue::List(a), CssValue::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| exactly_equal(a, b))
        }
        _ => false,
    }
}

fn hash_value<H: Hasher>(value: &CssValue, state: &mut H) {
    std::mem::discriminant(value).hash(state);
    match value {
        CssValue::None | CssValue::Zero | CssValue::Initial | CssValue::Inherit | CssValue::Comma => {}
        CssValue::Color(color) => color.exact_parts().hash(state),
        CssValue::Number(number, kind) => {
            number.to_bits().hash(state);
            kind.hash(state);
        }
        CssValue::Percentage(number) => number.to_bits().hash(state),
        CssValue::String(text) => text.hash(state),
        CssValue::Unit(number, unit) => {
            number.to_bits().hash(state);
            unit.hash(state);
        }
        CssValue::Function(name, args) => {
            name.hash(state);
            args.len().hash(state);
            for arg in args {
                hash_value(arg, state);
            }
        }
        CssValue::List(values) => {
            values.len().hash(state);
            for value in values {
                hash_value(value, state);
            }
        }
    }
}

/// The distinct values seen while parsing one stylesheet.
#[derive(Default)]
pub(crate) struct ValuePool {
    values: HashMap<ExactValue, Arc<CssValue>>,
}

impl ValuePool {
    /// The shared form of `value`: the one already pooled if this value has been seen, otherwise
    /// this one, pooled.
    pub(crate) fn intern(&mut self, value: CssValue) -> Arc<CssValue> {
        let value = Arc::new(value);
        let key = ExactValue(Arc::clone(&value));
        Arc::clone(self.values.entry(key).or_insert(value))
    }

    /// How many distinct values the pool holds, for tests and diagnostics.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::CssColor;

    #[test]
    fn the_same_value_written_twice_is_one_allocation() {
        let mut pool = ValuePool::default();
        let first = pool.intern(CssValue::String("none".into()));
        let second = pool.intern(CssValue::String("none".into()));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn values_that_differ_stay_apart() {
        let mut pool = ValuePool::default();
        pool.intern(CssValue::Unit(1.0, "px".into()));
        pool.intern(CssValue::Unit(1.0, "em".into()));
        pool.intern(CssValue::Unit(2.0, "px".into()));
        assert_eq!(pool.len(), 3);
    }

    #[test]
    fn colours_are_pooled_by_what_they_hold_not_by_what_they_look_like() {
        let mut pool = ValuePool::default();
        // `PartialEq` calls these the same colour - the difference is far under half a channel -
        // but they are not the same value, and the one that survived would be the one written
        // back to the page.
        let one = pool.intern(CssValue::Color(CssColor::srgb(10.0, 20.0, 30.0, 255.0)));
        let other = pool.intern(CssValue::Color(CssColor::srgb(10.000_1, 20.0, 30.0, 255.0)));
        assert!(!Arc::ptr_eq(&one, &other));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn nesting_is_compared_all_the_way_down() {
        let mut pool = ValuePool::default();
        let list = |unit: f64| {
            CssValue::List(vec![
                CssValue::Unit(unit, "px".into()),
                CssValue::String("solid".into()),
            ])
        };
        let first = pool.intern(list(1.0));
        let same = pool.intern(list(1.0));
        let different = pool.intern(list(2.0));
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &different));
    }
}
