mod css_system;
mod document;
mod layouter;

use crate::css3::CssSystem;
use crate::document::Document;
use crate::html5::Html5Parser;
use std::fmt::Debug;

pub use css_system::*;
pub use document::*;
pub use layouter::*;

/// Compile-time description of the engine components a client wires together.
///
/// A client (browser, headless tool, test) defines a single zero-sized marker type and
/// implements this trait once, naming every component as an associated type. The engine is
/// generic over `C: ModuleConfiguration`, so the whole component set is resolved and checked
/// at compile time — there is no runtime registry. Naming a component implies a dependency on
/// the crate that provides it (you cannot name `CairoBackend` without compiling Cairo in).
///
/// The narrow `Has*` view traits (e.g. [`HasCssSystem`]) are **derived automatically** from a
/// `ModuleConfiguration` via blanket impls below; they are never implemented by hand. Subsystem
/// signatures keep using the narrow `Has*` bounds, and any `C: ModuleConfiguration` satisfies
/// them.
///
/// Additional components (font system, layouter, network stack, render backend, compositor) are
/// added as associated types here as each is wired into the engine.
pub trait ModuleConfiguration: Clone + Debug + PartialEq + 'static {
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
