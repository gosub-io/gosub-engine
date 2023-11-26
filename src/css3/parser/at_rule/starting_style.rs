use crate::css3::node::Node;
use crate::css3::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_starting_style(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_starting_style");
        Ok(Node::new_cdo())
    }
}
