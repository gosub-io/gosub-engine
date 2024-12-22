use core::fmt::{Debug, Formatter};
use gosub_interface::node::DocTypeDataType;
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
    #[must_use]
    pub fn new(name: &str, pub_identifier: &str, sys_identifier: &str) -> Self {
        Self {
            name: name.to_owned(),
            pub_identifier: pub_identifier.to_owned(),
            sys_identifier: sys_identifier.to_owned(),
        }
    }
}

impl DocTypeDataType for DocTypeData {
    fn name(&self) -> &str {
        &self.name
    }

    fn pub_identifier(&self) -> &str {
        &self.pub_identifier
    }

    fn sys_identifier(&self) -> &str {
        &self.sys_identifier
    }
}
