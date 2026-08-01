mod css_system;
mod document;

use crate::css3::CssSystem;
use crate::document::Document;
use crate::html5::Html5Parser;
use std::fmt::Debug;

pub use css_system::*;
pub use document::*;

/// Compile-time description of the engine components a client wires together.
pub trait ModuleConfiguration: Clone + Debug + PartialEq + Send + Sync + 'static {
    /// CSS parser and property system.
    type CssSystem: CssSystem;

    /// DOM storage. In practice always `DocumentImpl`; present for type plumbing, not as a real
    /// swap point.
    type Document: Document<Self>;

    /// HTML5 tokeniser and tree builder.
    type HtmlParser: Html5Parser<Self>;
}

impl<C: ModuleConfiguration> HasCssSystem for C {
    type CssSystem = <C as ModuleConfiguration>::CssSystem;
}

impl<C: ModuleConfiguration> HasDocument for C {
    type Document = <C as ModuleConfiguration>::Document;
}

impl<C: ModuleConfiguration> HasHtmlParser for C {
    type HtmlParser = <C as ModuleConfiguration>::HtmlParser;
}
