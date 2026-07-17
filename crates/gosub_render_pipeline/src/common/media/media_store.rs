use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::{
    DecodedMedia, Image, Media, MediaDecoderRegistry, MediaId, MediaImage, MediaSvg, MediaType, Svg,
};
use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use url::Url;

const DEFAULT_SVG_ID: MediaId = MediaId::new(0);
const DEFAULT_IMAGE_ID: MediaId = MediaId::new(1);
const FIRST_FREE_IMAGE_ID: u64 = 100;

const DEFAULT_SVG_DATA: &[u8] = include_bytes!("../../../resources/not-found.svg");
const DEFAULT_IMAGE_DATA: &[u8] = include_bytes!("../../../resources/default-image.png");

/// Result of a non-blocking media request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRequest {
    /// The media is loaded and available under this id.
    Ready(MediaId),
    /// The media is being fetched in the background; try again after a reflow.
    Pending,
}

/// Keeps all loaded media in memory so it can be referenced by MediaId.
pub struct MediaStore {
    pub entries: RwLock<HashMap<MediaId, Arc<Media>>>,
    /// Keyed by hash(src)
    pub cache: RwLock<HashMap<Sha256Hash, MediaId>>,
    /// Hashes of resources currently being fetched in the background (dedupes in-flight requests)
    pending: RwLock<HashSet<Sha256Hash>>,
    /// Set whenever a background fetch lands, so the engine knows a reflow is needed
    completed: AtomicBool,
    /// Next media ID (atomic to prevent allocation races)
    next_id: AtomicU64,
    /// Compiled-in placeholder returned when an SVG is missing or failed to load
    default_svg: Arc<Media>,
    /// Compiled-in placeholder returned when an image is missing or failed to load
    default_image: Arc<Media>,
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
            pending: RwLock::new(HashSet::new()),
            completed: AtomicBool::new(false),
            next_id: AtomicU64::new(FIRST_FREE_IMAGE_ID),
            default_svg,
            default_image,
            decoders,
        }
    }

    /// Non-blocking media load: cached hits return `Ready`, otherwise a background fetch (deduped
    /// per src) starts and `Pending` is returned without blocking layout. On completion the
    /// `completed` flag rises and the engine's [`take_completed`](Self::take_completed) poll
    /// triggers a reflow. Takes `&Arc<Self>` so the fetch thread can share the store.
    pub fn request_media(self: &Arc<Self>, src: &str) -> MediaRequest {
        let h = hash_from_string(src);

        if let Some(media_id) = self.cache.read().get(&h) {
            return MediaRequest::Ready(*media_id);
        }

        // Register as in-flight; if another request already owns this hash, just report Pending.
        if !self.pending.write().insert(h) {
            return MediaRequest::Pending;
        }

        let store = Arc::clone(self);
        let src_owned = src.to_string();
        let spawned = std::thread::Builder::new().name("media-fetch".into()).spawn(move || {
            // `load_media` handles caching, and caches the placeholder on failure so a dead URL
            // is never re-fetched. We only need to clear the in-flight marker and signal completion.
            let _ = store.load_media(&src_owned);
            store.pending.write().remove(&h);
            store.completed.store(true, Ordering::Relaxed);
        });

        if spawned.is_err() {
            // Couldn't spawn - drop the in-flight marker so a later attempt can retry.
            self.pending.write().remove(&h);
        }

        MediaRequest::Pending
    }

    /// Returns and clears the "background fetch completed" flag; `true` means the engine should
    /// re-lay-out the page to pick up the new media.
    pub fn take_completed(&self) -> bool {
        self.completed.swap(false, Ordering::Relaxed)
    }

    /// Shared by the data, source and inline decode paths.
    fn decode_media(&self, src: &str, mime: Option<&str>, data: &[u8]) -> anyhow::Result<Media> {
        match self.decoders.decode(mime, data) {
            Ok(DecodedMedia::Raster(img)) => Ok(Media::image(src, img)),
            Ok(DecodedMedia::Vector(tree)) => Ok(Media::svg(src, Svg::new(*tree))),
            Err(e) => Err(anyhow::anyhow!("Failed to decode media from '{}': {}", src, e)),
        }
    }

    /// Loads `src` into the store, caching by src so repeat calls never reload. Fetch/decode
    /// failures cache the placeholder id, so a dead URL skips the network on later calls.
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
        // Another thread may have inserted while we were loading - don't overwrite
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

        // The hint only steers the raster-vs-vector choice; the registry re-sniffs the actual
        // format from the bytes anyway.
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

    /// Rasterize an SVG background to a `w`×`h` raster tile and return its media id, so a tiled
    /// `background-image: url(x.svg)` reuses the raster tiling path. Cached per (svg id, w, h) so
    /// it renders once. Returns `None` if the source is not an SVG or the pixmap can't allocate.
    pub fn svg_raster_tile(&self, svg_media_id: MediaId, w: u32, h: u32) -> Option<MediaId> {
        if w == 0 || h == 0 {
            return None;
        }
        let key = hash_from_string(&format!("svg-tile:{}:{}x{}", svg_media_id.as_u64(), w, h));
        if let Some(id) = self.cache.read().get(&key) {
            return Some(*id);
        }
        let media = self.get(svg_media_id, MediaType::Svg);
        let Media::Svg(svg) = &*media else {
            return None;
        };
        let image = render_svg_tree_to_image(&svg.svg.tree, w, h)?;
        let media_id = self.allocate_media_id();
        self.entries
            .write()
            .insert(media_id, Arc::new(Media::image("gosub://svg-tile", image)));
        self.cache.write().insert(key, media_id);
        Some(media_id)
    }

    fn load_media_from_source(&self, src: &str) -> anyhow::Result<MediaId> {
        log::debug!("Loading non-cached media from path: {}", src);
        // `data:` URIs carry the bytes inline - decode them directly instead of going to the network.
        let media = if let Some(rest) = src.strip_prefix("data:") {
            let (mime, bytes) = decode_data_uri(rest)?;
            self.decode_media(src, mime.as_deref(), &bytes)?
        } else {
            let (content_type, raw_data) = self.fetch_resource(src)?;
            self.decode_media(src, content_type.as_deref(), &raw_data)?
        };

        let media_id = self.allocate_media_id();
        self.entries.write().insert(media_id, Arc::new(media));

        Ok(media_id)
    }

    /// Falls back to the default image if `media_id` is missing or is not an image.
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

    /// Falls back to the default SVG if `media_id` is missing or is not an SVG.
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

    /// True for the built-in fallback placeholders, so callers can avoid propagating a
    /// placeholder's intrinsic pixel dimensions into layout.
    pub fn is_placeholder(&self, media_id: MediaId) -> bool {
        media_id == DEFAULT_IMAGE_ID || media_id == DEFAULT_SVG_ID
    }

    pub fn update_svg(&self, media_id: MediaId, media: Arc<Media>) {
        let mut entries = self.entries.write();
        entries.insert(media_id, media);
    }

    /// Falls back to `media_type`'s default resource if `media_id` does not exist.
    pub fn get(&self, media_id: MediaId, media_type: MediaType) -> Arc<Media> {
        let entries = self.entries.read();

        match entries.get(&media_id) {
            Some(media) => media.clone(),
            None => self.default_media(media_type),
        }
    }

    fn default_media(&self, media_type: MediaType) -> Arc<Media> {
        match media_type {
            MediaType::Svg => Arc::clone(&self.default_svg),
            MediaType::Image => Arc::clone(&self.default_image),
        }
    }

    /// Blocking fetch returning the raw `Content-Type` header and body. Classification is left to
    /// the decoder registry, which treats the content type as a hint only.
    fn fetch_resource(&self, src: &str) -> anyhow::Result<(Option<String>, Bytes)> {
        let url = Url::parse(src)?;
        let response = gosub_sonar::net::simple::sync_fetch(&url)?;

        if !response.is_ok() {
            anyhow::bail!("HTTP {} fetching resource", response.status);
        }

        let content_type = response.headers.get("content-type").cloned();
        let raw_bytes = Bytes::from(response.body);

        Ok((content_type, raw_bytes))
    }
}

/// Decodes a `data:` URI body (everything after `data:`) in its `[<mime>][;base64],<data>` form.
/// The MIME is a hint only - the decoder registry re-sniffs the real format.
fn decode_data_uri(rest: &str) -> anyhow::Result<(Option<String>, Vec<u8>)> {
    let (meta, data) = rest
        .split_once(',')
        .ok_or_else(|| anyhow::anyhow!("malformed data URI: missing ','"))?;

    let is_base64 = meta.rsplit(';').any(|t| t.eq_ignore_ascii_case("base64"));
    let mime = meta.split(';').next().filter(|s| !s.is_empty()).map(str::to_string);

    let bytes = if is_base64 {
        use base64::Engine;
        // Data URIs may contain whitespace/newlines; strip it before decoding.
        let cleaned: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(cleaned.as_bytes())
            .map_err(|e| anyhow::anyhow!("invalid base64 in data URI: {e}"))?
    } else {
        // Percent-decode a plain (text) payload, e.g. `data:image/svg+xml,<svg …>`.
        percent_decode(data)
    };

    Ok((mime, bytes))
}

/// Minimal `%XX` percent-decoding for plain `data:` URI payloads. Invalid escapes are left as-is.
fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Rasterize a `usvg` tree to a straight-alpha RGBA [`Image`] of `w`×`h` px (scaling the tree's
/// intrinsic size to fit). Returns `None` if the pixmap can't be allocated.
fn render_svg_tree_to_image(tree: &resvg::usvg::Tree, w: u32, h: u32) -> Option<Image> {
    let size = tree.size();
    let (iw, ih) = (size.width().max(1.0), size.height().max(1.0));
    let (sx, sy) = (w as f32 / iw, h as f32 / ih);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    resvg::render(tree, resvg::usvg::Transform::from_scale(sx, sy), &mut pixmap.as_mut());

    // tiny_skia pixmaps are premultiplied RGBA; the store wants straight (unpremultiplied) alpha.
    let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for px in pixmap.pixels() {
        let c = px.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Image::new_rgba8(w, h, rgba).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

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
    /// store with its real dimensions - not collapse to the fallback placeholder.
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
