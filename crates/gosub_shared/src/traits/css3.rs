use crate::async_executor::WasmNotSend;
use crate::document::DocumentHandle;
use crate::errors::CssResult;
use crate::node::NodeId;
use crate::traits::document::Document;
use crate::traits::render_tree::RenderTree;
use crate::traits::ParserConfig;
use std::fmt::Debug;

/// Defines the origin of the stylesheet (or declaration)
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CssOrigin {
    /// Browser/user agent defined stylesheets
    UserAgent,
    /// Author defined stylesheets that are linked or embedded in the HTML files
    Author,
    /// User defined stylesheets that will override the author and user agent stylesheets (for instance, custom user styles or extensions)
    User,
}

/// The CssSystem trait is a trait that defines all things CSS3 that are used by other non-css3 crates. This is the main trait that
/// is used to parse CSS3 files. It contains sub elements like the Stylesheet trait that is used in for instance the Document trait.
pub trait CssSystem: Clone + 'static {
    type Stylesheet: CssStylesheet;

    type PropertyMap: CssPropertyMap<Property = Self::Property>;

    type Property: CssProperty;

    /// Parses a string into a CSS3 stylesheet
    fn parse_str(str: &str, config: ParserConfig, origin: CssOrigin, source_url: &str) -> CssResult<Self::Stylesheet>;

    /// Returns the properties of a node
    /// If `None` is returned, the node is not renderable
    fn properties_from_node<D: Document<Self>>(
        node: &D::Node,
        sheets: &[Self::Stylesheet],
        handle: DocumentHandle<D, Self>,
        id: NodeId,
    ) -> Option<Self::PropertyMap>;

    fn inheritance<T: RenderTree<Self>>(tree: &mut T);

    fn load_default_useragent_stylesheet() -> Self::Stylesheet;
}

pub trait CssStylesheet: PartialEq {
    /// Returns the origin of the stylesheet
    fn origin(&self) -> CssOrigin;

    /// Returns the source URL of the stylesheet
    fn url(&self) -> &str;
}

pub trait CssPropertyMap: Default + Debug + WasmNotSend {
    type Property: CssProperty;

    fn insert_inherited(&mut self, name: &str, value: Self::Property);

    fn get(&self, name: &str) -> Option<&Self::Property>;

    fn get_mut(&mut self, name: &str) -> Option<&mut Self::Property>;

    fn make_dirty(&mut self);

    fn iter(&self) -> impl Iterator<Item = (&str, &Self::Property)> + '_;

    fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut Self::Property)> + '_;

    fn make_clean(&mut self);
    fn is_dirty(&self) -> bool;
}
pub trait CssProperty: Debug + Sized {
    type Value: CssValue;

    fn compute_value(&mut self); // this should probably be removed

    fn unit_to_px(&self) -> f32;

    fn as_string(&self) -> Option<&str>;
    fn as_percentage(&self) -> Option<f32>;
    fn as_unit(&self) -> Option<(f32, &str)>;
    fn as_color(&self) -> Option<(f32, f32, f32, f32)>;

    fn parse_color(&self) -> Option<(f32, f32, f32, f32)>;

    fn as_number(&self) -> Option<f32>;
    fn as_list(&self) -> Option<Vec<Self::Value>>;

    fn is_none(&self) -> bool;
}

pub trait CssValue: Sized {
    fn unit_to_px(&self) -> f32;

    fn as_string(&self) -> Option<&str>;
    fn as_percentage(&self) -> Option<f32>;
    fn as_unit(&self) -> Option<(f32, &str)>;
    fn as_color(&self) -> Option<(f32, f32, f32, f32)>;
    fn as_number(&self) -> Option<f32>;
    fn as_list(&self) -> Option<Vec<Self>>;

    fn is_comma(&self) -> bool;

    fn is_none(&self) -> bool;
}
