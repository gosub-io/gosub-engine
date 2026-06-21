use crate::stylesheet::CssValue;

/// Resolve the math comparison functions `clamp()`, `min()` and `max()` over their
/// length/number arguments.
///
/// Each operand is reduced to pixels via [`CssValue::unit_to_px`] (which handles px, em,
/// rem, the absolute physical units and the viewport units), so mixed-unit expressions
/// like `clamp(2.3rem, 5.5vw, 3.6rem)` collapse to a single pixel value. The result is
/// returned as a `Unit(_, "px")`.
///
/// Returns `None` (leaving the function unresolved) when any operand is not a plain
/// length we can compare — e.g. a percentage (no containing block here) or a nested
/// function — so callers can fall back to the original token.
pub fn resolve_math(func: &str, values: &[CssValue]) -> Option<CssValue> {
    // Drop the comma separators that the parser keeps between arguments.
    let operands: Option<Vec<f32>> = values
        .iter()
        .filter(|v| !matches!(v, CssValue::Comma))
        .map(operand_px)
        .collect();
    let operands = operands?;
    if operands.is_empty() {
        return None;
    }

    let px = match func {
        "min" => operands.iter().copied().fold(f32::INFINITY, f32::min),
        "max" => operands.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        // clamp(MIN, VAL, MAX) == max(MIN, min(VAL, MAX)).
        "clamp" => {
            let [min, val, max] = operands[..] else {
                return None;
            };
            val.min(max).max(min)
        }
        _ => return None,
    };

    Some(CssValue::Unit(px, "px".to_string()))
}

/// Reduce a single math operand to pixels, or `None` if it is not a comparable length.
fn operand_px(value: &CssValue) -> Option<f32> {
    match value {
        CssValue::Unit(..) => Some(value.unit_to_px()),
        CssValue::Number(n) => Some(*n),
        CssValue::Zero => Some(0.0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: f32, u: &str) -> CssValue {
        CssValue::Unit(v, u.to_string())
    }

    #[test]
    fn clamp_picks_the_middle_when_in_range() {
        // clamp(2.3rem=36.8, 5.5vw=70.4, 3.6rem=57.6) -> max(36.8, min(70.4, 57.6)) = 57.6
        let args = vec![
            unit(2.3, "rem"),
            CssValue::Comma,
            unit(5.5, "vw"),
            CssValue::Comma,
            unit(3.6, "rem"),
        ];
        assert_eq!(resolve_math("clamp", &args), Some(unit(57.6, "px")));
    }

    #[test]
    fn clamp_floors_to_min() {
        // clamp(100px, 10px, 200px) -> 100px
        let args = vec![
            unit(100.0, "px"),
            CssValue::Comma,
            unit(10.0, "px"),
            CssValue::Comma,
            unit(200.0, "px"),
        ];
        assert_eq!(resolve_math("clamp", &args), Some(unit(100.0, "px")));
    }

    #[test]
    fn min_and_max() {
        let args = vec![unit(10.0, "px"), CssValue::Comma, unit(20.0, "px")];
        assert_eq!(resolve_math("min", &args), Some(unit(10.0, "px")));
        assert_eq!(resolve_math("max", &args), Some(unit(20.0, "px")));
    }

    #[test]
    fn unresolvable_operand_bails() {
        let args = vec![unit(10.0, "px"), CssValue::Comma, CssValue::Percentage(50.0)];
        assert_eq!(resolve_math("min", &args), None);
    }
}
