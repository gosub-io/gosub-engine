use core::fmt::{Debug, Formatter};
use std::fmt;

#[derive(PartialEq, Clone)]
/// Data structure for document nodes
pub struct DocTypeData {
    pub name: String,
    pub pub_identifier: String,
    pub sys_identifier: String,
}

impl Default for DocTypeData {
    fn default() -> Self {
        Self::new("", "", "")
    }
}

impl Debug for DocTypeData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("DocTypeData");
        debug.finish()
    }
}

impl DocTypeData {
    pub(crate) fn new(name: &str, pub_identifier: &str, sys_identifier: &str) -> Self {
        DocTypeData {
            name: name.to_string(),
            pub_identifier: pub_identifier.to_string(),
            sys_identifier: sys_identifier.to_string(),
        }
    }
}
