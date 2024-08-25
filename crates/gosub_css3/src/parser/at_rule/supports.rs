use crate::node::{Node, NodeType};
use crate::{Css3, Error};

impl Css3<'_> {
    pub fn parse_at_rule_supports_prelude(&mut self) -> Result<Node, Error> {
        log::trace!("parse_at_rule_supports_prelude");

        let loc = self.tokenizer.current_location();

        // @todo: parse supports condition
        let value = self.consume_raw_condition()?;

        Ok(Node::new(NodeType::Raw { value }, loc))
    }
}

#[cfg(test)]
mod tests {
    use crate::walker::Walker;
    use gosub_shared::byte_stream::{ByteStream, Encoding};

    #[test]
    fn test_parse_at_rule_supports_prelude() {
        let mut stream = ByteStream::new(Encoding::UTF8, None);
        stream.read_from_str("(display: flex)", Some(Encoding::UTF8));
        stream.close();

        let mut parser = crate::Css3::new(&mut stream);
        let node = parser.parse_at_rule_supports_prelude().unwrap();

        let w = Walker::new(&node);
        assert_eq!(w.walk_to_string(), "[Raw] (display: flex)\n")
    }
}
