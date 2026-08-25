use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::{
    DecodedImage, DecodedMedia, Image, Media, MediaDecoderRegistry, MediaId, MediaImage, MediaSvg, MediaType, Svg,
};
use bytes::Bytes;
use gosub_interface::media_decoder::{BrokeredDecode, ImageDecoder};
use gosub_interface::resource_loader::{LoadError, NoResourceLoader, ResourceLoader};
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
    /// Fetch inline on the calling thread instead of spawning `media-fetch` threads.
    /// A sandboxed renderer cannot spawn (its seccomp filter has no `clone`; a spawn is
    /// SIGSYS, not `Err`), and an inline fetch means one layout pass instead of fetch-then-reflow.
    synchronous_fetch: AtomicBool,
    /// Next media ID (atomic to prevent allocation races)
    next_id: AtomicU64,
    /// Compiled-in placeholder returned when an SVG is missing or failed to load
    default_svg: Arc<Media>,
    /// Compiled-in placeholder returned when an image is missing or failed to load
    default_image: Arc<Media>,
    decoders: MediaDecoderRegistry,
    /// How remote media is fetched. The store holds a loader rather than reaching
    /// for the network itself, so layout carries no network capability.
    loader: Arc<dyn ResourceLoader>,
    /// Where raster decoding happens. `None` decodes in this process, which is
    /// the default; the engine installs one to move it out.
    decoder: Option<Arc<dyn ImageDecoder>>,
    /// The bytes each raster image was decoded from, so its pixels can be let
    /// go of under [`decoded_budget`](Self::set_decoded_budget) and decoded
    /// again when next drawn - what bounds a page of photographs.
    encoded: RwLock<HashMap<MediaId, EncodedSource>>,
    /// Most recent use per media id, for choosing what to let go of.
    recent: parking_lot::Mutex<Recency>,
    /// Decoded raster bytes to keep resident; 0 (the default) keeps everything.
    decoded_budget: AtomicU64,
}

/// What a raster image can be decoded from again.
struct EncodedSource {
    src: String,
    mime: Option<String>,
    bytes: Bytes,
    /// Its size for layout, so asking is never a reason to decode.
    intrinsic: (u32, u32),
}

#[derive(Default)]
struct Recency {
    tick: u64,
    last_used: HashMap<MediaId, u64>,
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

    /// A store with no network: `data:` URIs still decode, remote sources do not
    /// load. For tests and measurement-only layout passes; the engine builds its
    /// store with [`with_loader`](Self::with_loader).
    pub fn new() -> MediaStore {
        Self::with_loader(Arc::new(NoResourceLoader))
    }

    /// A store that pulls remote media through `loader`.
    pub fn with_loader(loader: Arc<dyn ResourceLoader>) -> MediaStore {
        Self::with_loader_and_decoder(loader, None)
    }

    /// A store that also decodes raster images through `decoder` rather than in
    /// this process. See [`ImageDecoder`].
    pub fn with_loader_and_decoder(
        loader: Arc<dyn ResourceLoader>,
        decoder: Option<Arc<dyn ImageDecoder>>,
    ) -> MediaStore {
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
            synchronous_fetch: AtomicBool::new(false),
            next_id: AtomicU64::new(FIRST_FREE_IMAGE_ID),
            default_svg,
            default_image,
            decoders,
            loader,
            decoder,
            encoded: RwLock::new(HashMap::new()),
            recent: parking_lot::Mutex::new(Recency::default()),
            decoded_budget: AtomicU64::new(0),
        }
    }

    /// Fetch inline instead of spawning `media-fetch` threads (see the field). Set once,
    /// before the store is shared with a context that cannot thread (the fork server sets
    /// it before renderers are forked from it).
    pub fn set_synchronous_fetch(&self, on: bool) {
        self.synchronous_fetch.store(on, Ordering::Relaxed);
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

        // Synchronous mode: fetch on the calling thread. `load_media` caches even
        // failures (as the placeholder), so the lookup below normally succeeds.
        if self.synchronous_fetch.load(Ordering::Relaxed) {
            let loaded = self.load_media(src);
            self.pending.write().remove(&h);
            if loaded.is_ok() {
                self.completed.store(true, Ordering::Relaxed);
            }
            return match self.cache.read().get(&h) {
                Some(media_id) => MediaRequest::Ready(*media_id),
                None => MediaRequest::Pending,
            };
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

    /// Decoded bytes held for loaded media (RGBA for images; an estimate for SVG trees).
    pub fn resident_bytes(&self) -> usize {
        self.entries
            .read()
            .values()
            .map(|media| match &**media {
                Media::Image(image) => image.image.as_raw().len(),
                Media::Svg(_) => 64 * 1024,
            })
            .sum()
    }

    /// Bound the decoded raster pixels kept resident. Above it, the least
    /// recently used images give up their pixels (their encoded bytes stay)
    /// and are decoded again on their next use. A long-lived process with a
    /// fixed memory limit needs this; `0` keeps everything.
    pub fn set_decoded_budget(&self, bytes: u64) {
        self.decoded_budget.store(bytes, Ordering::Relaxed);
    }

    fn touch(&self, media_id: MediaId) {
        let mut recent = self.recent.lock();
        recent.tick += 1;
        let tick = recent.tick;
        recent.last_used.insert(media_id, tick);
    }

    /// Let go of least recently used decoded images until the resident total
    /// fits the budget; `keep` (just decoded, about to be used) survives.
    fn enforce_decoded_budget(&self, keep: MediaId) {
        let budget = self.decoded_budget.load(Ordering::Relaxed);
        if budget == 0 {
            return;
        }
        let encoded = self.encoded.read();
        let mut entries = self.entries.write();
        let mut resident: u64 = entries
            .iter()
            .filter(|(id, _)| encoded.contains_key(id))
            .map(|(_, media)| match &**media {
                Media::Image(image) => image.image.as_raw().len() as u64,
                Media::Svg(_) => 0,
            })
            .sum();
        if resident <= budget {
            return;
        }
        let mut recent = self.recent.lock();
        let mut candidates: Vec<(u64, MediaId)> = entries
            .keys()
            .filter(|id| **id != keep && encoded.contains_key(id))
            .map(|id| (recent.last_used.get(id).copied().unwrap_or(0), *id))
            .collect();
        candidates.sort_unstable_by_key(|(tick, id)| (*tick, id.as_u64()));
        for (_, id) in candidates {
            if resident <= budget {
                break;
            }
            if let Some(media) = entries.remove(&id) {
                if let Media::Image(image) = &*media {
                    resident = resident.saturating_sub(image.image.as_raw().len() as u64);
                }
            }
            recent.last_used.remove(&id);
        }
    }

    /// Decode an image whose pixels were let go of, from the bytes kept for it.
    fn revive(&self, media_id: MediaId) -> Option<Arc<Media>> {
        let (src, mime, bytes) = {
            let encoded = self.encoded.read();
            let source = encoded.get(&media_id)?;
            (source.src.clone(), source.mime.clone(), source.bytes.clone())
        };
        let media = match self.decode_media(&src, mime.as_deref(), &bytes) {
            Ok(media) => Arc::new(media),
            Err(e) => {
                log::warn!("could not decode '{src}' again: {e}");
                return None;
            }
        };
        self.entries.write().insert(media_id, Arc::clone(&media));
        self.touch(media_id);
        self.enforce_decoded_budget(media_id);
        Some(media)
    }

    /// Drop every loaded media once more than `budget_bytes` is held, keeping the
    /// compiled-in placeholders. All-or-nothing on purpose: a long-lived process
    /// (a resident renderer) calls this between pages, when nothing it holds is
    /// known to be needed again and re-fetching what is comes from the broker's
    /// cache anyway. Returns how many bytes were released.
    pub fn trim(&self, budget_bytes: usize) -> usize {
        let held = self.resident_bytes();
        if held <= budget_bytes {
            return 0;
        }
        let mut entries = self.entries.write();
        let mut cache = self.cache.write();
        entries.retain(|id, _| *id == DEFAULT_SVG_ID || *id == DEFAULT_IMAGE_ID);
        cache.clear();
        self.encoded.write().clear();
        *self.recent.lock() = Recency::default();
        held
    }

    /// Returns and clears the "background fetch completed" flag; `true` means the engine should
    /// re-lay-out the page to pick up the new media.
    pub fn take_completed(&self) -> bool {
        self.completed.swap(false, Ordering::Relaxed)
    }

    /// Shared by the data, source and inline decode paths.
    fn decode_media(&self, src: &str, mime: Option<&str>, data: &[u8]) -> anyhow::Result<Media> {
        if let Some(decoder) = &self.decoder {
            match decoder.decode(mime, data) {
                Ok(BrokeredDecode::Raster(raster)) => {
                    // Length is checked against the dimensions rather than
                    // trusted: the producer may be a compromised decoder.
                    let image = DecodedImage::new_rgba8(raster.width, raster.height, raster.rgba.to_vec())
                        .map_err(|e| anyhow::anyhow!("brokered decode of '{}' returned bad pixels: {}", src, e))?;
                    return Ok(Media::image(src, image));
                }
                // Vector data: fall through and parse it here.
                Ok(BrokeredDecode::Vector) => {}
                Err(e) => {
                    log::debug!("brokered decode of '{src}' did not produce an image ({e}); decoding locally");
                }
            }
        }

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
            // Not here yet, not a failure: nothing is cached, and the loader's
            // owner re-renders once the bytes arrive.
            Err(e) if is_pending(&e) => return Err(e),
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
        let (mime, bytes) = if let Some(rest) = src.strip_prefix("data:") {
            let (mime, bytes) = decode_data_uri(rest)?;
            (mime, Bytes::from(bytes))
        } else {
            self.fetch_resource(src)?
        };
        let media = self.decode_media(src, mime.as_deref(), &bytes)?;

        let media_id = self.allocate_media_id();
        let intrinsic = match &media {
            Media::Image(image) => Some((image.image.intrinsic_width(), image.image.intrinsic_height())),
            Media::Svg(_) => None,
        };
        self.entries.write().insert(media_id, Arc::new(media));
        if let Some(intrinsic) = intrinsic {
            // Kept so the pixels can be given up and brought back (see `set_decoded_budget`).
            self.encoded.write().insert(
                media_id,
                EncodedSource {
                    src: src.to_string(),
                    mime,
                    bytes,
                    intrinsic,
                },
            );
            self.touch(media_id);
            self.enforce_decoded_budget(media_id);
        }

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

    /// A raster image's size for layout, without decoding it: resident or not.
    /// `None` for SVGs, placeholders and unknown ids.
    pub fn image_intrinsic_size(&self, media_id: MediaId) -> Option<(u32, u32)> {
        if let Some(source) = self.encoded.read().get(&media_id) {
            return Some(source.intrinsic);
        }
        match self.entries.read().get(&media_id).map(|m| &**m) {
            Some(Media::Image(image)) => Some((image.image.intrinsic_width(), image.image.intrinsic_height())),
            _ => None,
        }
    }

    /// Whether every pixel of a raster image is transparent - known only
    /// while its pixels are resident; an image let go of under the budget
    /// answers `false` rather than being decoded for the question.
    pub fn is_fully_transparent(&self, media_id: MediaId) -> bool {
        match self.entries.read().get(&media_id).map(|m| &**m) {
            Some(Media::Image(image)) => {
                image.image.intrinsic_width() > 0 && image.image.as_raw().as_chunks::<4>().0.iter().all(|px| px[3] == 0)
            }
            _ => false,
        }
    }

    /// Falls back to `media_type`'s default resource if `media_id` does not exist.
    /// An image whose pixels were let go of under the decoded budget is decoded
    /// again here.
    pub fn get(&self, media_id: MediaId, media_type: MediaType) -> Arc<Media> {
        let resident = self.entries.read().get(&media_id).cloned();
        if let Some(media) = resident {
            if self.decoded_budget.load(Ordering::Relaxed) != 0 {
                self.touch(media_id);
            }
            return media;
        }
        self.revive(media_id).unwrap_or_else(|| self.default_media(media_type))
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
        let response = self.loader.load(&url)?;

        if !response.is_ok() {
            anyhow::bail!("HTTP {} fetching resource", response.status);
        }

        Ok((response.content_type, response.body))
    }
}

/// Whether a load error says "not yet" rather than "no".
fn is_pending(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|cause| matches!(cause.downcast_ref::<LoadError>(), Some(LoadError::Pending)))
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

#[cfg(test)]
mod decoded_budget_tests {
    use super::*;

    fn data_uri(width: u32, height: u32, seed: u8) -> String {
        use image::ImageEncoder;
        let pixels = vec![seed; (width * height * 4) as usize];
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
            .expect("encode");
        let mut b64 = String::new();
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for chunk in png.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
            for i in 0..4 {
                b64.push(if i <= chunk.len() {
                    ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        format!("data:image/png;base64,{b64}")
    }

    #[test]
    fn decoded_pixels_are_bounded_and_come_back_on_use() {
        let store = Arc::new(MediaStore::new());
        // Each image is 100x100x4 = 40 000 bytes; room for two.
        store.set_decoded_budget(90_000);
        store.set_synchronous_fetch(true);
        let ids: Vec<MediaId> = (0..4u8)
            .map(|seed| match store.request_media(&data_uri(100, 100, seed)) {
                MediaRequest::Ready(id) => id,
                MediaRequest::Pending => panic!("synchronous load should be ready"),
            })
            .collect();

        let resident = |store: &MediaStore| -> usize {
            let entries = store.entries.read();
            ids.iter().filter(|id| entries.contains_key(id)).count()
        };
        assert!(resident(&store) <= 2, "budget must hold: {} resident", resident(&store));
        // The first ones loaded were the ones let go of.
        assert!(!store.entries.read().contains_key(&ids[0]));

        // Using an evicted image decodes it again, at its own size, and the
        // hash→id mapping still answers Ready for its source.
        let Media::Image(back) = &*store.get(ids[0], MediaType::Image) else {
            panic!("expected an image");
        };
        assert_eq!((back.image.width(), back.image.height()), (100, 100));
        assert_eq!(back.image.as_raw()[0], 0);
        assert!(store.entries.read().contains_key(&ids[0]));
        assert!(resident(&store) <= 2);
        assert!(matches!(store.request_media(&data_uri(100, 100, 0)), MediaRequest::Ready(id) if id == ids[0]));
    }
}
