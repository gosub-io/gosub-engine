use crate::config::HasCssSystem;
use crate::document::Document;
use crate::html5::Html5Parser;
use std::fmt::Debug;

pub trait HasDocument: HasCssSystem + Sized + Clone + Debug + PartialEq + 'static {
    type Document: Document<Self>;
}

pub trait HasHtmlParser: HasDocument {
    type HtmlParser: Html5Parser<Self>;
}
