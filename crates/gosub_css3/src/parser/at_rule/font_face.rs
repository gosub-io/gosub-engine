use crate::node::Node;
use crate::Css3;
use gosub_shared::errors::{CssError, CssResult};

impl Css3<'_> {
    pub fn parse_at_rule_font_face_block(&mut self) -> CssResult<Node> {
        log::trace!("parse_at_rule_font_face_block");

        Err(CssError::with_location(
            "@font-face block not yet implemented",
            self.tokenizer.current_location(),
        ))
    }
}
