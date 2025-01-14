use crate::config::HasDocument;
use gosub_shared::byte_stream::{ByteStream, Location};

use gosub_shared::types::{ParseError, Result};

pub trait Html5Parser<C: HasDocument> {
    type Options: ParserOptions;

    fn parse(stream: &mut ByteStream, doc: &mut C::Document, opts: Option<Self::Options>) -> Result<Vec<ParseError>>;

    #[allow(clippy::type_complexity)]
    fn parse_fragment(
        stream: &mut ByteStream,
        doc: &mut C::Document,
        context_node: C::Node,
        options: Option<Self::Options>,
        start_location: Location,
    ) -> Result<Vec<ParseError>>;
}

pub trait ParserOptions {
    fn new(scripting: bool) -> Self;
}
