use std::ops::AddAssign;
use std::sync::Arc;
use crate::common::hash::{hash_from_string, Sha256Hash};
use crate::common::media::Image;
use crate::common::media::Svg;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaId(u64);

impl MediaId {
    pub const fn new(val: u64) -> Self {
        Self(val)
    }
}

impl AddAssign<i32> for MediaId {
    fn add_assign(&mut self, rhs: i32) {
        self.0 += rhs as u64;
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
    /// Source of the SVG
    src: String,
    /// Hash of the source
    hash: Sha256Hash,
    /// Actual SVG tree
    pub svg: Svg,
}

#[allow(unused)]
#[derive(Debug, Clone)]
pub struct MediaImage {
    src: String,
    hash: Sha256Hash,
    pub image: Image,
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
            hash: hash_from_string(&src),
            svg
        }))
    }

    pub fn image(src: &str, image: Image) -> Self {
        Media::Image(Arc::new(MediaImage {
            src: src.to_string(),
            hash: hash_from_string(&src),
            image
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
