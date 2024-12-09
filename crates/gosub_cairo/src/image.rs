use std::sync::Arc;
use image::{DynamicImage, GenericImageView};

use gosub_shared::render_backend::geo::FP;
use gosub_shared::render_backend::Image as TImage;

#[derive(Clone)]
enum Format {
    Rgba8,
}

#[derive(Clone)]
pub struct Image {
    pub image: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub format: Format,
}

impl TImage for Image {
    fn new(size: (FP, FP), data: Vec<u8>) -> Self {
        Image {
            image: Arc::new(data),
            width: size.0 as u32,
            height: size.1 as u32,
            format: Format::Rgba8,
        }
    }

    fn from_img(img: DynamicImage) -> Self {
        let (width, height) = img.dimensions();

        let data = img.into_rgba8().into_raw();
        Image {
            image: Arc::new(data),
            width,
            height,
            format: Format::Rgba8,
        }
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}
