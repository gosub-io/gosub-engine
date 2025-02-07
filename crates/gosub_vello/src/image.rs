use std::sync::Arc;

use gosub_interface::render_backend::Image as TImage;
use gosub_shared::geo::FP;
use image::{DynamicImage, GenericImageView};
use vello::peniko::ImageFormat;
use vello::peniko::{Blob, Image as VelloImage};

#[derive(Clone)]
pub struct Image(pub(crate) VelloImage);

impl From<VelloImage> for Image {
    fn from(image: VelloImage) -> Self {
        Image(image)
    }
}

impl TImage for Image {
    fn new(size: (FP, FP), data: Vec<u8>) -> Self {
        let blob = Blob::new(Arc::new(data));

        Image(VelloImage::new(blob, ImageFormat::Rgba8, size.0 as u32, size.1 as u32))
    }

    fn from_img(img: DynamicImage) -> Self {
        let (width, height) = img.dimensions();

        let data = img.into_rgba8().into_raw();
        let blob = Blob::new(Arc::new(data));

        Image(VelloImage::new(blob, ImageFormat::Rgba8, width, height))
    }

    fn width(&self) -> u32 {
        self.0.width
    }

    fn height(&self) -> u32 {
        self.0.height
    }
}
