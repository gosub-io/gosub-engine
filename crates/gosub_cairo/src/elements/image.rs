use image::{DynamicImage, GenericImageView};
use std::sync::Arc;

use gosub_shared::render_backend::geo::FP;
use gosub_shared::render_backend::Image as TImage;

#[derive(Clone, Debug)]
pub enum GsFormat {
    Rgba8,
}

#[derive(Clone, Debug)]
pub struct GsImage {
    pub image: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub format: GsFormat,
}

impl TImage for GsImage {
    fn new(size: (FP, FP), data: Vec<u8>) -> Self {
        GsImage {
            image: Arc::new(data),
            width: size.0 as u32,
            height: size.1 as u32,
            format: GsFormat::Rgba8,
        }
    }

    fn from_img(img: DynamicImage) -> Self {
        let (width, height) = img.dimensions();

        let data = img.into_rgba8().into_raw();
        GsImage {
            image: Arc::new(data),
            width,
            height,
            format: GsFormat::Rgba8,
        }
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}
