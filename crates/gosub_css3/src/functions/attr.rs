use crate::stylesheet::CssValue;
use gosub_interface::config::HasDocument;
use gosub_interface::node::{ElementDataType, Node};

// Probably this shouldn't quite be in gosub_css3
#[allow(dead_code)]
pub fn resolve_attr<C: HasDocument>(values: &[CssValue], node: &C::Node) -> Vec<CssValue> {
    let Some(attr_name) = values.first().map(std::string::ToString::to_string) else {
        return vec![];
    };

    let ty = values.get(1).cloned();

    let Some(data) = node.get_element_data() else {
        return vec![];
    };

    let Some(attr_value) = data.attribute(&attr_name) else {
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
