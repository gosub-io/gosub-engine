use crate::stylesheet::CssValue;
use gosub_shared::traits::config::HasDocument;
use std::collections::HashMap;

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct VariableEnvironment {
    pub values: HashMap<String, CssValue>,
}

#[allow(dead_code)]
impl VariableEnvironment {
    pub fn get<C: HasDocument>(&self, name: &str, _doc: &C::Document, _node: &C::Node) -> Option<CssValue> {
        let mut current = Some(self);

        while let Some(env) = current {
            if let Some(value) = env.values.get(name) {
                return Some(value.clone());
            }

            current = None;

            //TODO: give node a variable env
            // let node = doc.get_parent(node);
            // current =  node.get_variable_env();
        }

        None
    }
}

#[allow(dead_code)]
pub fn resolve_var<C: HasDocument>(values: &[CssValue], doc: &C::Document, node: &C::Node) -> Vec<CssValue> {
    let Some(name) = values.first().map(|v| {
        let mut str = v.to_string();

        if str.starts_with("--") {
            str.remove(0);
            str.remove(0);
        }

        str
    }) else {
        return vec![];
    };

    // let environment = doc.get_variable_env(node);

    let environment = VariableEnvironment::default(); //TODO: get from node

    let Some(value) = environment.get::<C>(&name, doc, node) else {
        let Some(default) = values.get(1).cloned() else {
            return vec![];
        };

        return vec![default];
    };

    vec![value.clone()]
}
