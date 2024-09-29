use crate::byte_stream::{ByteStream, Location};
use crate::document::DocumentHandle;
use crate::traits::css3::CssSystem;
use crate::traits::document::Document;

use crate::types::{ParseError, Result};

pub trait Html5Parser<C: CssSystem> {
    type Document: Document<C>;

    type Options: ParserOptions;

    fn parse(
        stream: &mut ByteStream,
        doc: DocumentHandle<Self::Document, C>,
        opts: Option<Self::Options>,
    ) -> Result<Vec<ParseError>>;

    #[allow(clippy::type_complexity)]
    fn parse_fragment(
        stream: &mut ByteStream,
        doc: DocumentHandle<Self::Document, C>,
        context_node: &<Self::Document as Document<C>>::Node,
        options: Option<Self::Options>,
        start_location: Location,
    ) -> Result<Vec<ParseError>>;
}

pub trait ParserOptions {
    fn new(scripting: bool) -> Self;
}
