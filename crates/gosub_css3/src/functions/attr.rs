use crate::stylesheet::CssValue;
use crate::tokenizer::NumberKind;
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

/// Resolve one `attr( <attr-name> <attr-type>? , <declaration-value>? )` against the element's
/// attributes (css-values-5 §12.1).
///
/// The substitution is the attribute's value read as `<attr-type>`, or the fallback when the
/// attribute is absent or does not read that way. With no type the value is a *string*, which is
/// why `content: attr(title)` works and `width: attr(data-w)` does not: a string is not a length,
/// so that declaration is invalid. A type says otherwise - `attr(data-w px)` is a length.
///
/// This used to read the second argument as the fallback, which is where the type goes, and to
/// parse the attribute as a CSS value whatever was asked for. So `attr(data-w, 10px)` took the
/// comma itself as its fallback, and an untyped `attr()` smuggled a length into any property.
///
/// `type( <syntax> )` is not handled: the value parser drops the declaration before it gets
/// here. An empty return is the guaranteed-invalid value, and the caller drops the declaration.
pub fn resolve_attr<C: HasDocument>(values: &[CssValue], doc: &C::Document, id: NodeId) -> Vec<CssValue> {
    // `attr()` takes at most one comma, and everything after it is the fallback - which may be
    // several values (`attr(data-x px, 1px solid red)`) or empty (`attr(data-x,)`).
    let comma = values.iter().position(|value| matches!(value, CssValue::Comma));
    let (head, fallback) = match comma {
        Some(at) => (&values[..at], Some(&values[at + 1..])),
        None => (values, None),
    };

    let Some(attr_name) = head.first().map(std::string::ToString::to_string) else {
        return vec![];
    };
    let attr_type = head.get(1);

    // No fallback written means the empty string for an untyped `attr()`, and nothing at all -
    // an invalid declaration - for a typed one.
    let use_fallback = || match fallback {
        Some(values) if !values.is_empty() => values.to_vec(),
        _ if attr_type.is_none() => vec![CssValue::String(String::new())],
        _ => vec![],
    };

    let Some(attr_value) = doc.attribute(id, &attr_name) else {
        return use_fallback();
    };
    let attr_value = attr_value.trim();

    let Some(attr_type) = attr_type else {
        // Untyped: the attribute's value as a string, verbatim.
        return vec![CssValue::String(attr_value.to_string())];
    };
    let CssValue::String(attr_type) = attr_type else {
        return use_fallback();
    };

    // Every type but the string ones reads the attribute as a number, and the type says what
    // that number measures.
    if attr_type.eq_ignore_ascii_case("string") || attr_type.eq_ignore_ascii_case("raw-string") {
        return vec![CssValue::String(attr_value.to_string())];
    }
    let Ok(number) = attr_value.parse::<f64>() else {
        return use_fallback();
    };
    if attr_type.eq_ignore_ascii_case("number") {
        return vec![CssValue::Number(number, NumberKind::Number)];
    }
    if attr_type.eq_ignore_ascii_case("integer") {
        if number.fract() != 0.0 {
            return use_fallback();
        }
        return vec![CssValue::Number(number, NumberKind::Integer)];
    }
    if attr_type == "%" || attr_type.eq_ignore_ascii_case("percentage") {
        return vec![CssValue::Percentage(number)];
    }
    if is_known_unit(attr_type) {
        return vec![CssValue::Unit(number, attr_type.cow_to_ascii_lowercase().into_owned())];
    }
    use_fallback()
}

/// The units `<attr-unit>` allows: the dimension units of css-values, which is every unit this
/// engine converts. A word that is not one of them is not a type, and the fallback applies.
fn is_known_unit(unit: &str) -> bool {
    const UNITS: [&str; 30] = [
        "em", "rem", "ex", "rex", "cap", "rcap", "ch", "rch", "ic", "ric", "lh", "rlh", "vw", "vh", "vi", "vb", "vmin",
        "vmax", "cm", "mm", "q", "in", "pt", "pc", "px", "deg", "grad", "rad", "turn", "fr",
    ];
    UNITS.iter().any(|known| unit.eq_ignore_ascii_case(known))
}
