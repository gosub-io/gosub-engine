use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::{DecodedMedia, Media, MediaDecoderRegistry, MediaId, MediaImage, MediaSvg, MediaType, Svg};
use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use url::Url;

const DEFAULT_SVG_ID: MediaId = MediaId::new(0);
const DEFAULT_IMAGE_ID: MediaId = MediaId::new(1);
const FIRST_FREE_IMAGE_ID: u64 = 100;

const DEFAULT_SVG_DATA: &[u8] = include_bytes!("../../../resources/not-found.svg");
const DEFAULT_IMAGE_DATA: &[u8] = include_bytes!("../../../resources/default-image.png");

/// Media store keeps all the loaded media in memory so it can be referenced by its MediaID
pub struct MediaStore {
    /// List of all media
    pub entries: RwLock<HashMap<MediaId, Arc<Media>>>,
    /// List of all images by hash(src)
    pub cache: RwLock<HashMap<Sha256Hash, MediaId>>,
    /// Next media ID (atomic to prevent allocation races)
    next_id: AtomicU64,
    /// Compiled-in placeholder returned when an SVG is missing or failed to load
    default_svg: Arc<Media>,
    /// Compiled-in placeholder returned when an image is missing or failed to load
    default_image: Arc<Media>,
    /// Pluggable decoders that turn raw bytes into [`Media`]
    decoders: MediaDecoderRegistry,
}

impl Default for MediaStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaStore {
    fn allocate_media_id(&self) -> MediaId {
        MediaId::new(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn new() -> MediaStore {
        let decoders = MediaDecoderRegistry::with_defaults();

        #[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in asset, exercised by every pipeline test
        let default_svg = match decoders
            .decode(Some("image/svg+xml"), DEFAULT_SVG_DATA)
            .expect("Failed to decode default svg")
        {
            DecodedMedia::Vector(tree) => Arc::new(Media::svg("gosub://default/svg", Svg::new(*tree))),
            DecodedMedia::Raster(_) => unreachable!("default svg decoded as a raster image"),
        };

        #[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in asset, exercised by every pipeline test
        let default_image = match decoders
            .decode(None, DEFAULT_IMAGE_DATA)
            .expect("Failed to decode default image")
        {
            DecodedMedia::Raster(img) => Arc::new(Media::image("gosub://default/image", img)),
            DecodedMedia::Vector(_) => unreachable!("default image decoded as an svg"),
        };

        let entries = HashMap::from([
            (DEFAULT_SVG_ID, Arc::clone(&default_svg)),
            (DEFAULT_IMAGE_ID, Arc::clone(&default_image)),
        ]);

        MediaStore {
            entries: RwLock::new(entries),
            cache: RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(FIRST_FREE_IMAGE_ID),
            default_svg,
            default_image,
            decoders,
        }
    }

    /// Decode `data` (with an optional MIME hint) through the registry and wrap the result in a
    /// [`Media`]. Shared by the data, source and inline decode paths.
    fn decode_media(&self, src: &str, mime: Option<&str>, data: &[u8]) -> anyhow::Result<Media> {
        match self.decoders.decode(mime, data) {
            Ok(DecodedMedia::Raster(img)) => Ok(Media::image(src, img)),
            Ok(DecodedMedia::Vector(tree)) => Ok(Media::svg(src, Svg::new(*tree))),
            Err(e) => Err(anyhow::anyhow!("Failed to decode media from '{}': {}", src, e)),
        }
    }

    /// Load the given media from src into the media store, and return the media ID. Will also store the media(id) in cache
    /// so the next call with the same src will return the same media ID without reloading.
    ///
    /// If the resource cannot be fetched or decoded, the default placeholder for the detected media
    /// type is returned and the failure is cached so subsequent calls with the same URL skip the
    /// network entirely.
    pub fn load_media(&self, src: &str) -> anyhow::Result<MediaId> {
        let h = hash_from_string(src);
        let cache = self.cache.read();
        if let Some(media_id) = cache.get(&h) {
            log::debug!("Loading cached media from path: {}", src);
            return Ok(*media_id);
        }
        drop(cache);

        let result = self.load_media_from_source(src);

        let media_id = match result {
            Ok(media_id) => media_id,
            Err(e) => {
                log::warn!("Failed to load media from '{}': {}", src, e);
                // Cache the failure as the default image placeholder so the same URL is
                // never re-fetched in this session (avoids repeated blocking I/O).
                let fallback_id = DEFAULT_IMAGE_ID;
                let mut cache = self.cache.write();
                cache.entry(h).or_insert(fallback_id);
                return Ok(fallback_id);
            }
        };

        let mut cache = self.cache.write();
        // Another thread may have inserted while we were loading — don't overwrite
        cache.entry(h).or_insert(media_id);

        Ok(media_id)
    }

    pub fn load_media_from_data(&self, media_type: MediaType, data: &[u8]) -> anyhow::Result<MediaId> {
        let h = hash_from_data(data);
        {
            let cache = self.cache.read();
            if let Some(media_id) = cache.get(&h) {
                log::debug!("Loading cached media from data");
                return Ok(*media_id);
            }
        }

        // The caller knows the kind of media, so pass it as a MIME hint. The registry still
        // re-sniffs the actual format from the bytes, so the hint only steers the raster-vs-vector
        // choice; raster data is detected by its magic bytes.
        let mime = match media_type {
            MediaType::Svg => Some("image/svg+xml"),
            MediaType::Image => None,
        };
        let media = self.decode_media("gosub://data", mime, data)?;

        let media_id = self.allocate_media_id();
        self.entries.write().insert(media_id, Arc::new(media));
        self.cache.write().insert(h, media_id);

        Ok(media_id)
    }

    fn load_media_from_source(&self, src: &str) -> anyhow::Result<MediaId> {
        log::debug!("Loading non-cached media from path: {}", src);
        let (content_type, raw_data) = self.fetch_resource(src)?;

        let media = self.decode_media(src, content_type.as_deref(), &raw_data)?;

        let media_id = self.allocate_media_id();
        self.entries.write().insert(media_id, Arc::new(media));

        Ok(media_id)
    }

    /// Returns a media image. If the media is not an image or does not exist, it will return the default media image id
    pub fn get_image(&self, media_id: MediaId) -> Arc<MediaImage> {
        let media = self.get(media_id, MediaType::Image);
        match &*media {
            Media::Image(media_image) => media_image.clone(),
            _ => {
                log::warn!("Media {:?} is not an image, returning default", media_id);
                let default = self.default_media(MediaType::Image);
                match &*default {
                    Media::Image(img) => img.clone(),
                    _ => unreachable!("Default image is not an image"),
                }
            }
        }
    }

    /// Returns a media svg. If the media is not an svg or does not exist, it will return the default media svg id
    pub fn get_svg(&self, media_id: MediaId) -> Arc<MediaSvg> {
        let media = self.get(media_id, MediaType::Svg);
        match &*media {
            Media::Svg(media_svg) => media_svg.clone(),
            _ => {
                log::warn!("Media {:?} is not an SVG, returning default", media_id);
                let default = self.default_media(MediaType::Svg);
                match &*default {
                    Media::Svg(svg) => svg.clone(),
                    _ => unreachable!("Default SVG is not an SVG"),
                }
            }
        }
    }

    /// Returns true when `media_id` is one of the built-in fallback placeholders
    /// (used when a resource failed to load). Callers can use this to avoid
    /// propagating the placeholder's intrinsic pixel dimensions into layout.
    pub fn is_placeholder(&self, media_id: MediaId) -> bool {
        media_id == DEFAULT_IMAGE_ID || media_id == DEFAULT_SVG_ID
    }

    pub fn update_svg(&self, media_id: MediaId, media: Arc<Media>) {
        let mut entries = self.entries.write();
        entries.insert(media_id, media);
    }

    /// Returns a media resource. If the media does not exist, it will return the default media resource as specified by the media_type
    pub fn get(&self, media_id: MediaId, media_type: MediaType) -> Arc<Media> {
        let entries = self.entries.read();

        match entries.get(&media_id) {
            Some(media) => media.clone(),
            None => self.default_media(media_type),
        }
    }

    /// Returns the default media resource for the given media type
    fn default_media(&self, media_type: MediaType) -> Arc<Media> {
        match media_type {
            MediaType::Svg => Arc::clone(&self.default_svg),
            MediaType::Image => Arc::clone(&self.default_image),
        }
    }

    /// Fetch a resource from the web (or local file system, depending on the src) and return the
    /// raw `Content-Type` header (if any) together with the body bytes. Format classification is
    /// left to the decoder registry, which treats the content type as a hint and falls back to
    /// magic-byte sniffing. This is blocking.
    fn fetch_resource(&self, src: &str) -> anyhow::Result<(Option<String>, Bytes)> {
        let url = Url::parse(src)?;
        let response = gosub_net::net::simple::sync_fetch(&url)?;

        if !response.is_ok() {
            anyhow::bail!("HTTP {} fetching resource", response.status);
        }

        let content_type = response.headers.get("content-type").cloned();
        let raw_bytes = Bytes::from(response.body);

        Ok((content_type, raw_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    /// Encode a small solid image into `format`, returning the raw bytes.
    fn encode(format: ImageFormat) -> Vec<u8> {
        let rgba = DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 4, Rgba([200, 100, 50, 255])));
        let mut buf = Cursor::new(Vec::new());
        match format {
            // JPEG has no alpha channel, so encode from an RGB view.
            ImageFormat::Jpeg => DynamicImage::ImageRgb8(rgba.to_rgb8())
                .write_to(&mut buf, format)
                .expect("encode jpeg"),
            _ => rgba.write_to(&mut buf, format).expect("encode image"),
        }
        buf.into_inner()
    }

    /// Each of PNG/JPEG/GIF must decode through `load_media_from_data` and land in the
    /// store with its real dimensions — not collapse to the fallback placeholder.
    #[test]
    fn decodes_png_jpeg_gif() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Gif] {
            let store = MediaStore::new();
            let bytes = encode(format);

            let media_id = store
                .load_media_from_data(MediaType::Image, &bytes)
                .unwrap_or_else(|e| panic!("{format:?} failed to load: {e}"));

            assert!(
                !store.is_placeholder(media_id),
                "{format:?} fell back to the placeholder instead of decoding"
            );

            let img = store.get_image(media_id);
            assert_eq!(img.image.width(), 8, "{format:?} width");
            assert_eq!(img.image.height(), 4, "{format:?} height");
        }
    }

    /// SVG data must decode through `load_media_from_data` into a retained SVG (not the
    /// placeholder), so it can be re-rasterized at any size.
    #[test]
    fn decodes_svg_from_data() {
        let store = MediaStore::new();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="blue"/></svg>"#;

        let media_id = store
            .load_media_from_data(MediaType::Svg, svg)
            .unwrap_or_else(|e| panic!("svg failed to load: {e}"));

        assert!(!store.is_placeholder(media_id), "svg fell back to the placeholder");
        let svg = store.get_svg(media_id);
        let size = svg.svg.tree.size();
        assert_eq!((size.width() as u32, size.height() as u32), (20, 10));
    }
}
