mod css_system;
mod document;
mod layouter;

use crate::font::HasFontManager;
pub use css_system::*;
pub use document::*;
pub use layouter::*;

pub trait ModuleConfiguration:
    Sized + HasCssSystem + HasDocument + HasHtmlParser + HasFontManager + HasLayouter
{
}
