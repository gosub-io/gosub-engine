use crate::config::HasDocument;
use crate::node::Location;
use crate::types::{ParseError, Result};

pub trait Html5Parser<C: HasDocument> {
    type Options: ParserOptions;
    type Stream;

    fn parse(stream: &mut Self::Stream, doc: &mut C::Document, opts: Option<Self::Options>) -> Result<Vec<ParseError>>;

    #[allow(clippy::type_complexity)]
    fn parse_fragment(
        stream: &mut Self::Stream,
        doc: &mut C::Document,
        context_node: C::Node,
        options: Option<Self::Options>,
        start_location: Location,
    ) -> Result<Vec<ParseError>>;
}

pub trait ParserOptions {
    fn new(scripting: bool) -> Self;
}
