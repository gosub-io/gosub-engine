use gosub_css3::stylesheet::CssValue;
use gosub_html5::node::Node;

pub fn resolve_attr(values: &[CssValue], node: &Node) -> Vec<CssValue> {
    let Some(attr_name) = values.first().map(|v| v.to_string()) else {
        return vec![];
    };

    let ty = values.get(1).cloned();

    let Some(attr_value) = node.get_attribute(&attr_name) else {
        let _default_value = values.get(2).cloned();

        if let Some(ty) = ty {
            return vec![ty];
        }

        return vec![];
    };

    let Ok(value) = CssValue::parse_str(attr_value) else {
        return vec![];
    };

    vec![value]
}
