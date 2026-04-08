mod image;
mod svg;

mod media;
mod media_store;

pub use media::Media;
pub use media::MediaImage;
pub use media::MediaType;
pub use media::MediaSvg;
pub use media::MediaId;

pub use svg::Svg;
pub use image::Image;

pub use media_store::get_media_store;