use crate::stylesheet::CssValue;
use std::collections::HashMap;

pub fn resolve_var(values: &[CssValue], custom_props: &HashMap<String, CssValue>) -> Vec<CssValue> {
    let Some(name) = values.first().map(|v| v.to_string()) else {
        return vec![];
    };

    if let Some(value) = custom_props.get(&name) {
        return vec![value.clone()];
    }

    // Variable not found — use the fallback if provided
    if let Some(fallback) = values.get(1).cloned() {
        return vec![fallback];
    }

    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_defined_variable() {
        let mut props = HashMap::new();
        props.insert("--color".to_string(), CssValue::String("red".to_string()));

        let args = vec![CssValue::String("--color".to_string())];
        assert_eq!(resolve_var(&args, &props), vec![CssValue::String("red".to_string())]);
    }

    #[test]
    fn falls_back_when_undefined() {
        let props = HashMap::new();
        let args = vec![
            CssValue::String("--missing".to_string()),
            CssValue::String("blue".to_string()),
        ];
        assert_eq!(resolve_var(&args, &props), vec![CssValue::String("blue".to_string())]);
    }

    #[test]
    fn returns_empty_when_undefined_and_no_fallback() {
        let props = HashMap::new();
        let args = vec![CssValue::String("--missing".to_string())];
        assert_eq!(resolve_var(&args, &props), vec![]);
    }
}
