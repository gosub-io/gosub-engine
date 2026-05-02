use crate::stylesheet::CssValue;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

#[allow(dead_code)]
pub fn resolve_attr<C: HasDocument>(values: &[CssValue], doc: &C::Document, id: NodeId) -> Vec<CssValue> {
    let Some(attr_name) = values.first().map(std::string::ToString::to_string) else {
        return vec![];
    };

    let ty = values.get(1).cloned();

    let Some(attr_value) = doc.attribute(id, &attr_name) else {
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
