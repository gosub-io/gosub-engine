mod decoder;
mod image;
pub mod image_source;
mod svg;

#[allow(clippy::module_inception)]
mod media;
mod media_store;

pub(crate) use decoder::MAX_IMAGE_EDGE;
pub use decoder::{
    decodes_image_type, DecodedImage, DecodedMedia, ImageDecodeError, MediaDecoder, MediaDecoderRegistry, PixelBuffer,
    RasterDecoder, SvgDecoder, MAX_KEPT_PIXELS,
};

pub use media::Media;
pub use media::MediaId;
pub use media::MediaImage;
pub use media::MediaSvg;
pub use media::MediaType;

pub use image::Image;
pub use svg::{svg_raster_size, Svg};

pub use media_store::render_svg_tree_to_image;
pub use media_store::MediaRequest;
pub use media_store::{Acquired, MediaInitiator, MediaSource, MediaStore};
