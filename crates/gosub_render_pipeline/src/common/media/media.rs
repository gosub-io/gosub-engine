use crate::common::hash::{hash_from_string, Sha256Hash};
use crate::common::media::Image;
use crate::common::media::Svg;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaId(u64);

impl MediaId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for MediaId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "MediaId({})", self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaType {
    Svg,
    Image,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct MediaSvg {
    src: String,
    hash: Sha256Hash,
    pub svg: Svg,
}

#[allow(unused)]
#[derive(Clone)]
pub struct MediaImage {
    src: String,
    hash: Sha256Hash,
    pub image: Image,
    /// The pixels as the CPU rasterizers composite them, converted once (see
    /// [`Self::premultiplied_bgra`]).
    premultiplied: Arc<std::sync::OnceLock<Arc<Vec<u8>>>>,
}

/// Like the image's own `Debug`: the source and the sizes, never the pixel bytes.
impl std::fmt::Debug for MediaImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaImage")
            .field("src", &self.src)
            .field("image", &self.image)
            .field("premultiplied", &self.premultiplied.get().is_some())
            .finish()
    }
}

impl MediaImage {
    /// The image as premultiplied ARGB32 in host byte order (B, G, R, A on little-endian),
    /// tightly packed at four bytes per pixel: what Cairo paints from. Converted on first use
    /// and shared from then on. Doing it per paint cost a full pass over the image for every
    /// tile the image touched - a 3000 px hero background in a page re-rastered during a
    /// window drag paid that for each of the viewport's tiles, every size.
    pub fn premultiplied_bgra(&self) -> Arc<Vec<u8>> {
        Arc::clone(self.premultiplied.get_or_init(|| {
            let src = self.image.as_raw();
            let mut data = vec![0u8; src.len()];
            for (d, s) in data.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                let (r, g, b, a) = (s[0] as u32, s[1] as u32, s[2] as u32, s[3] as u32);
                d[0] = (b * a / 255) as u8;
                d[1] = (g * a / 255) as u8;
                d[2] = (r * a / 255) as u8;
                d[3] = a as u8;
            }
            Arc::new(data)
        }))
    }
}

#[derive(Clone)]
pub enum Media {
    Svg(Arc<MediaSvg>),
    Image(Arc<MediaImage>),
}

impl Media {
    pub fn svg(src: &str, svg: Svg) -> Self {
        Media::Svg(Arc::new(MediaSvg {
            src: src.to_string(),
            hash: hash_from_string(src),
            svg,
        }))
    }

    pub fn image(src: &str, image: Image) -> Self {
        Media::Image(Arc::new(MediaImage {
            src: src.to_string(),
            hash: hash_from_string(src),
            image,
            premultiplied: Arc::default(),
        }))
    }
}

impl std::fmt::Debug for Media {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Media::Svg(svg) => write!(f, "Media::Svg({:?})", svg),
            Media::Image(image) => write!(f, "Media::Image({:?})", image),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::media::decoder::DecodedImage;

    /// One straight-alpha RGBA pixel comes out premultiplied and byte-swapped
    /// to BGRA, and a second call hands back the same shared buffer.
    #[test]
    fn premultiplied_pixels_are_converted_once() {
        let image = DecodedImage::new_rgba8(1, 1, vec![200, 100, 50, 128]).unwrap();
        let Media::Image(media) = Media::image("a.png", image) else {
            unreachable!()
        };
        let first = media.premultiplied_bgra();
        assert_eq!(first.as_slice(), &[25, 50, 100, 128]);
        let second = media.premultiplied_bgra();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(format!("{media:?}").contains("premultiplied: true"));
    }
}
