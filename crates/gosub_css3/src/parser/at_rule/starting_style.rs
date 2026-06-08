use crate::node::Node;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    pub fn parse_at_rule_starting_style_block(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_starting_style_block");

        Err(CssError::with_location(
            "@starting-style block not yet implemented",
            self.tokenizer.current_location(),
        ))
    }
}
