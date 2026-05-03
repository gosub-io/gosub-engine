mod image;
mod svg;

#[allow(clippy::module_inception)]
mod media;
mod media_store;

pub use media::Media;
pub use media::MediaId;
pub use media::MediaImage;
pub use media::MediaSvg;
pub use media::MediaType;

pub use image::Image;
pub use svg::Svg;

pub use media_store::get_media_store;
