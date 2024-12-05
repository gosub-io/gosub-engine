use crate::stylesheet::CssValue;

#[allow(dead_code)]
pub fn resolve_calc(values: &[CssValue]) -> Vec<CssValue> {
    println!("Calc called with {values:?}");
    vec![CssValue::None]
}
