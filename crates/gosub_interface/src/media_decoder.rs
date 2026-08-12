//! Decoding an image somewhere other than here.

use std::fmt;

/// A decoded raster image in the one form that survives a process boundary:
/// dimensions plus tightly packed RGBA8.
#[derive(Clone)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// Exactly `width * height * 4` bytes. Callers must verify this rather than
    /// trust it - the producer may be a compromised decoder.
    pub rgba: bytes::Bytes,
}

impl fmt::Debug for RasterImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RasterImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// What a decoder made of the input.
#[derive(Debug, Clone)]
pub enum BrokeredDecode {
    Raster(RasterImage),
    /// Vector data, which the caller must parse itself - see the module docs.
    Vector,
}

/// Why a decode produced nothing.
#[derive(Debug, Clone)]
pub enum DecodeError {
    /// The bytes are not an image, or not one this decoder understands.
    Unsupported(String),
    /// Decoding was attempted and failed.
    Failed(String),
    /// The decoder did not finish in time and was abandoned. Distinct from
    /// [`Failed`](DecodeError::Failed) because it usually means a hostile or
    /// pathological input rather than a merely malformed one.
    TimedOut,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Unsupported(why) => write!(f, "unsupported image: {why}"),
            DecodeError::Failed(why) => write!(f, "decode failed: {why}"),
            DecodeError::TimedOut => write!(f, "decode timed out"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes image bytes on the caller's behalf.
pub trait ImageDecoder: Send + Sync + fmt::Debug {
    /// Decode `bytes`. `mime` is a hint from the response and may be absent or
    /// wrong; a decoder is expected to sniff rather than trust it.
    fn decode(&self, mime: Option<&str>, bytes: &[u8]) -> Result<BrokeredDecode, DecodeError>;
}
