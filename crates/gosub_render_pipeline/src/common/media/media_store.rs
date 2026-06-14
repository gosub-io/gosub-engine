use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::Svg;
use crate::common::media::{Media, MediaId, MediaImage, MediaSvg, MediaType};
use bytes::Bytes;
use file_type::FileType;
use parking_lot::RwLock;
use resvg::usvg;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use url::Url;

/// Return `usvg::Options` backed by a shared fontdb that has system fonts loaded.
///
/// The database is built once the first time this is called and then reused,
/// so system font discovery only happens once per process.
fn svg_options() -> usvg::Options<'static> {
    static FONTDB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fontdb = Arc::clone(FONTDB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    }));
    usvg::Options {
        fontdb,
        ..Default::default()
    }
}

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
        #[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in asset, exercised by every pipeline test
        let default_svg_tree =
            usvg::Tree::from_data(DEFAULT_SVG_DATA, &svg_options()).expect("Failed to load default svg");
        let default_svg = Arc::new(Media::svg("gosub://default/svg", Svg::new(default_svg_tree)));

        #[allow(clippy::expect_used)] // PANIC-SAFE: compiled-in asset, exercised by every pipeline test
        let default_image_data = image::load_from_memory(DEFAULT_IMAGE_DATA)
            .expect("Failed to load default image")
            .to_rgba8();
        let default_image = Arc::new(Media::image("gosub://default/image", default_image_data));

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
        let cache = self.cache.read();
        if let Some(media_id) = cache.get(&h) {
            log::debug!("Loading cached media from data");
            return Ok(*media_id);
        }
        drop(cache);

        let media_id = match media_type {
            MediaType::Svg => {
                let svg_tree = match usvg::Tree::from_data(data, &svg_options()) {
                    Ok(tree) => tree,
                    Err(_) => {
                        return Err(anyhow::anyhow!("Failed to parse SVG data"));
                    }
                };

                let media = Media::svg("gosub://data/svg", Svg::new(svg_tree));
                let media_id = self.allocate_media_id();

                let mut entries = self.entries.write();
                entries.insert(media_id, Arc::new(media));
                drop(entries);

                let mut cache = self.cache.write();
                cache.insert(h, media_id);

                media_id
            }
            MediaType::Image => {
                let img = match image::load_from_memory(data) {
                    Ok(img) => img,
                    Err(_) => {
                        return Err(anyhow::anyhow!("Failed to parse image data"));
                    }
                };

                let media = Media::image("gosub://data/image", img.to_rgba8());
                let media_id = self.allocate_media_id();

                let mut entries = self.entries.write();
                entries.insert(media_id, Arc::new(media));
                drop(entries);

                let mut cache = self.cache.write();
                cache.insert(h, media_id);

                media_id
            }
        };

        Ok(media_id)
    }

    fn load_media_from_source(&self, src: &str) -> anyhow::Result<MediaId> {
        log::debug!("Loading non-cached media from path: {}", src);
        let Ok((media_type, raw_data)) = self.fetch_resource(src) else {
            anyhow::bail!("Failed to fetch resource");
        };

        let media = match media_type {
            MediaType::Svg => {
                let svg_tree = match usvg::Tree::from_data(&raw_data, &svg_options()) {
                    Ok(tree) => tree,
                    Err(_) => {
                        return Err(anyhow::anyhow!("Failed to parse SVG data"));
                    }
                };

                Media::svg(src, Svg::new(svg_tree))
            }
            MediaType::Image => {
                let img = match image::load_from_memory(&raw_data) {
                    Ok(img) => img,
                    Err(_) => {
                        return Err(anyhow::anyhow!("Failed to parse image data"));
                    }
                };

                Media::image(src, img.to_rgba8())
            }
        };

        let media_id = self.allocate_media_id();

        let mut entries = self.entries.write();
        entries.insert(media_id, Arc::new(media));

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

    /// Fetch resource from the web (or local file system, depending on the src) and returns the media type and raw
    /// bytes. This is blocking.
    fn fetch_resource(&self, src: &str) -> anyhow::Result<(MediaType, bytes::Bytes)> {
        let url = Url::parse(src)?;
        let response = gosub_net::net::simple::sync_fetch(&url)?;

        if !response.is_ok() {
            anyhow::bail!("HTTP {} fetching resource", response.status);
        }

        // Detect through content type
        let detected_content_type =
            detect_content_type(response.headers.get("content-type").map(String::as_str).unwrap_or(""));

        // Detect through content bytes
        let raw_bytes = bytes::Bytes::from(response.body);
        let detected_file_type = detect_file_type(&raw_bytes);

        // When the declared content type and the file bytes disagree, trust the bytes.
        match (detected_file_type, detected_content_type) {
            (Some(file_type), _) => Ok((file_type, raw_bytes)),
            (None, Some(content_type)) => Ok((content_type, raw_bytes)),
            (None, None) => anyhow::bail!("Failed to detect media type"),
        }
    }
}

fn detect_file_type(data: &Bytes) -> Option<MediaType> {
    let ft = FileType::from_bytes(data);
    if let Some(media_type) = ft_detect(ft) {
        return Some(media_type);
    }

    // The `file_type` crate misses some raster magic numbers — notably GIF, which it reports
    // as `application/octet-stream`. Fall back to the `image` crate's own format sniffing,
    // which reliably recognises GIF/PNG/JPEG/WebP/etc. SVG is handled above (it's XML text,
    // not something `image` decodes), so this only ever adds raster formats.
    if image::guess_format(data).is_ok() {
        return Some(MediaType::Image);
    }

    None
}

fn detect_content_type(content_type: &str) -> Option<MediaType> {
    let ft = FileType::from_media_type(content_type);
    if ft.is_empty() {
        return None;
    }
    ft_detect(ft[0])
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

    /// The byte-sniffing detector must classify PNG/JPEG/GIF as raster images. If this
    /// returns `None`, `fetch_resource` would bail and the image silently becomes the
    /// broken-image placeholder.
    #[test]
    fn detects_raster_formats_from_bytes() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::Gif] {
            let bytes = Bytes::from(encode(format));
            assert_eq!(
                detect_file_type(&bytes),
                Some(MediaType::Image),
                "{format:?} bytes were not detected as a raster image"
            );
        }
    }

    /// Content-type header detection must also recognise the common raster mime types.
    #[test]
    fn detects_raster_formats_from_content_type() {
        for ct in ["image/gif", "image/png", "image/jpeg"] {
            assert_eq!(
                detect_content_type(ct),
                Some(MediaType::Image),
                "content-type {ct} was not detected as a raster image"
            );
        }
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
}

fn ft_detect(ft: &FileType) -> Option<MediaType> {
    // Scan *all* media types before deciding. The `file_type` crate lists several aliases
    // for SVG (e.g. `["image/SVG", "image/svg+xml"]`); a naive first-match-wins loop trips
    // the generic `image/` branch on the non-standard `image/SVG` alias and misclassifies
    // SVG as a raster image. So look for any SVG marker first, and only fall back to a
    // generic raster image if none is found. Comparisons are case-insensitive.
    let mut is_raster_image = false;
    for &mt in ft.media_types().iter() {
        if mt.eq_ignore_ascii_case("image/svg+xml") || mt.eq_ignore_ascii_case("image/svg") {
            return Some(MediaType::Svg);
        }
        if mt.len() >= 6 && mt[..6].eq_ignore_ascii_case("image/") {
            is_raster_image = true;
        }
    }

    is_raster_image.then_some(MediaType::Image)
}
