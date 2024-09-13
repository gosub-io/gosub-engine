use std::collections::HashMap;

use gosub_css3::stylesheet::CssValue;
use gosub_html5::node::Node;
use gosub_html5::parser::document::Document;

#[derive(Clone, Debug, Default)]
pub struct VariableEnvironment {
    pub values: HashMap<String, CssValue>,
}

impl VariableEnvironment {
    pub fn get(&self, name: &str, _doc: &Document, _node: &Node) -> Option<CssValue> {
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

pub fn resolve_var(values: &[CssValue], doc: &Document, node: &Node) -> Vec<CssValue> {
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

    let Some(value) = environment.get(&name, doc, node) else {
        let Some(default) = values.get(1).cloned() else {
            return vec![];
        };

        return vec![default];
    };

    vec![value.clone()]
}
