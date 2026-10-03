use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::{
    DecodedImage, DecodedMedia, Image, Media, MediaDecoderRegistry, MediaId, MediaImage, MediaSvg, MediaType, Svg,
};
use bytes::Bytes;
use gosub_interface::media_decoder::{BrokeredDecode, ImageDecoder};
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

/// Whoever can put a URL's bytes into the resource handoff.
///
/// The media store does not fetch. It asks for what it needs and waits for the bytes to
/// appear in [`gosub_shared::subresource`], which is where the engine's resource pipeline
/// leaves everything it fetches. That keeps every request on one path -- with the policy,
/// the cache and the network panel that come with it -- and is the only shape that survives
/// the fetching moving to another process.
pub trait MediaSource: Send + Sync {
    /// Claim `url` in the hand-off and start fetching it, unless someone already has, and
    /// say which scope the bytes will arrive under. Returns immediately; the bytes turn up
    /// in the hand-off, or do not.
    ///
    /// The scope is the page the request belongs to, which is what the hand-off keys its
    /// entries by: bytes fetched for one page are not an answer to another page's request
    /// for the same URL, because two documents can share a cookie jar without sharing a
    /// request context. [`Acquired::Unowned`] when there is no page to speak for, which is also
    /// before anything can ask for an image.
    ///
    /// Claiming and asking are one operation on purpose. Split in two, the page can change
    /// between them, and then the consumer waits under one scope while the bytes are
    /// deposited under another -- which reads as an image that took the timeout to fail.
    fn acquire(&self, url: &str) -> Acquired;
}

/// What a [`MediaSource`] did with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquired {
    /// The bytes arrive in the hand-off under this scope - or its entry there says they will
    /// not.
    Under(gosub_shared::subresource::Scope),
    /// Being fetched, but not to be waited for: the source's owner lays the page out again
    /// once the bytes land. Not a failure, so the store caches nothing for it.
    Later,
    /// No page to speak for (nothing has been loaded yet), so nothing is fetched.
    Unowned,
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
    /// Where to ask for bytes this store does not have. `None` in a store nobody has wired
    /// up -- every test in this crate, and any embedder that only loads media from data it
    /// already holds -- which then simply has nothing to load.
    source: RwLock<Option<Arc<dyn MediaSource>>>,
    /// Fetch inline on the calling thread instead of spawning `media-fetch` threads.
    /// A sandboxed renderer cannot spawn (its seccomp filter has no `clone`; a spawn is
    /// SIGSYS, not `Err`), and an inline fetch means one layout pass instead of fetch-then-reflow.
    synchronous_fetch: AtomicBool,
    /// Next media ID (atomic to prevent allocation races)
    next_id: AtomicU64,
    /// Compiled-in placeholder returned when an SVG is missing or failed to load. `None` only
    /// if the compiled-in asset itself would not decode, which is a bug in this repository and
    /// not something a page can cause - but it used to be an `expect`, so that bug reached a
    /// user as a browser that would not start rather than as a missing placeholder.
    default_svg: Option<Arc<Media>>,
    /// Compiled-in placeholder returned when an image is missing or failed to load. See
    /// [`MediaStore::default_svg`].
    default_image: Option<Arc<Media>>,
    decoders: MediaDecoderRegistry,
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

    pub fn new() -> MediaStore {
        Self::with_decoder(None)
    }

    /// A store that decodes raster images through `decoder` rather than in this
    /// process. See [`ImageDecoder`].
    pub fn with_decoder(decoder: Option<Arc<dyn ImageDecoder>>) -> MediaStore {
        let decoders = MediaDecoderRegistry::with_defaults();

        let default_svg = match decoders.decode(Some("image/svg+xml"), DEFAULT_SVG_DATA) {
            Ok(DecodedMedia::Vector(tree)) => Some(Arc::new(Media::svg("gosub://default/svg", Svg::new(*tree)))),
            Ok(DecodedMedia::Raster(_)) => {
                log::error!("the built-in placeholder SVG decoded as a raster image; there will be no SVG placeholder");
                None
            }
            Err(e) => {
                log::error!("the built-in placeholder SVG would not decode, so there will be none: {e:?}");
                None
            }
        };

        let default_image = match decoders.decode(None, DEFAULT_IMAGE_DATA) {
            Ok(DecodedMedia::Raster(img)) => Some(Arc::new(Media::image("gosub://default/image", img))),
            Ok(DecodedMedia::Vector(_)) => {
                log::error!("the built-in placeholder image decoded as an SVG; there will be no image placeholder");
                None
            }
            Err(e) => {
                log::error!("the built-in placeholder image would not decode, so there will be none: {e:?}");
                None
            }
        };

        let entries = [
            default_svg.as_ref().map(|media| (DEFAULT_SVG_ID, Arc::clone(media))),
            default_image
                .as_ref()
                .map(|media| (DEFAULT_IMAGE_ID, Arc::clone(media))),
        ]
        .into_iter()
        .flatten()
        .collect::<HashMap<_, _>>();

        MediaStore {
            entries: RwLock::new(entries),
            cache: RwLock::new(HashMap::new()),
            pending: RwLock::new(HashSet::new()),
            completed: AtomicBool::new(false),
            source: RwLock::new(None),
            synchronous_fetch: AtomicBool::new(false),
            next_id: AtomicU64::new(FIRST_FREE_IMAGE_ID),
            default_svg,
            default_image,
            decoders,
            decoder,
            encoded: RwLock::new(HashMap::new()),
            recent: parking_lot::Mutex::new(Recency::default()),
            decoded_budget: AtomicU64::new(0),
        }
    }

    /// Wire up where the store asks for bytes it does not have. See [`MediaSource`].
    pub fn set_source(&self, source: Arc<dyn MediaSource>) {
        *self.source.write() = Some(source);
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

    /// Bytes kept to decode raster images again (see `set_decoded_budget`),
    /// their source strings included.
    pub fn encoded_bytes(&self) -> usize {
        self.encoded
            .read()
            .values()
            .map(|source| source.bytes.len() + source.src.len())
            .sum()
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
        // Everything the store holds for its images, not just resident
        // pixels: the encoded bytes kept for re-decoding (and their source
        // strings - a data: URL is the image again) can outweigh pixels the
        // decoded budget already let go of, and nothing else bounds them.
        let held = self.resident_bytes() + self.encoded_bytes();
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

    /// Shared by the data, source and inline decode paths. With a decoder
    /// installed the bytes are never decoded here: a failure - including one
    /// the decoder could not even start on - is the image's failure, not a
    /// reason to decode locally after all.
    fn decode_media(&self, src: &str, mime: Option<&str>, data: &[u8]) -> anyhow::Result<Media> {
        // Pure CPU: the bytes are already in hand, whether they came from the network,
        // a data: URI or inline markup. Fetching is timed separately as net.fetch.image.
        let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::DecodeImage, src);

        if let Some(decoder) = &self.decoder {
            return match decoder.decode(mime, data) {
                Ok(BrokeredDecode::Raster(raster)) => {
                    // Length is checked against the dimensions rather than
                    // trusted: the producer may be a compromised decoder.
                    let image = DecodedImage::new_rgba8(raster.width, raster.height, raster.rgba.to_vec())
                        .map_err(|e| anyhow::anyhow!("brokered decode of '{}' returned bad pixels: {}", src, e))?
                        // The decoder may have kept fewer pixels than the image has; it
                        // still lays out at its real size (bounded by the client).
                        .with_intrinsic(raster.intrinsic_width, raster.intrinsic_height);
                    Ok(Media::image(src, image))
                }
                Err(e) => Err(anyhow::anyhow!("brokered decode of '{}' failed: {}", src, e)),
            };
        }

        self.decode_locally(src, mime, data)
    }

    /// Decode in this process with the registry, whatever decoder is installed.
    fn decode_locally(&self, src: &str, mime: Option<&str>, data: &[u8]) -> anyhow::Result<Media> {
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
        // SVG markup given here is inline `<svg>` (or the engine's own control
        // icons): it came in with the document, which this process parsed
        // anyway, and it must stay a vector - a brokered decode would come back
        // as a raster, which the inline-SVG layout path cannot size, at the cost
        // of a decoder process per element.
        let media = match media_type {
            MediaType::Svg => {
                let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::DecodeImage, "gosub://data");
                self.decode_locally("gosub://data", mime, data)?
            }
            MediaType::Image => self.decode_media("gosub://data", mime, data)?,
        };

        let media_id = self.allocate_media_id();
        self.entries.write().insert(media_id, Arc::new(media));
        self.cache.write().insert(h, media_id);

        Ok(media_id)
    }

    /// Rasterize an SVG background to a `w`x`h` raster tile and return its media id, so a tiled
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
        let svg = self.get_svg(svg_media_id)?;
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

        // A synchronous fetch runs on the layout thread (the resident renderer), where
        // decoding every image of a page as its bytes land costs seconds. Layout only needs
        // the size: take it from the header and leave the pixels to `get` on first paint -
        // which, with a raster window, most images off-screen never reach. The asynchronous
        // path decodes here as before: that is a background thread, and the pixels are wanted
        // by the reflow that follows.
        if self.synchronous_fetch.load(Ordering::Relaxed) {
            // Header parsing is decoding too: through the decoder when there is one.
            let intrinsic = match &self.decoder {
                Some(decoder) => decoder.dimensions(mime.as_deref(), &bytes).ok(),
                None => self.decoders.dimensions(mime.as_deref(), &bytes),
            };
            if let Some(intrinsic) = intrinsic {
                let media_id = self.allocate_media_id();
                self.encoded.write().insert(
                    media_id,
                    EncodedSource {
                        src: src.to_string(),
                        mime,
                        bytes,
                        intrinsic,
                    },
                );
                return Ok(media_id);
            }
        }

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

    /// Falls back to the default image if `media_id` is missing or is not an image, and to
    /// `None` if there is no default either.
    pub fn get_image(&self, media_id: MediaId) -> Option<Arc<MediaImage>> {
        if let Some(Media::Image(media_image)) = self.get(media_id, MediaType::Image).as_deref() {
            return Some(media_image.clone());
        }
        log::warn!("Media {media_id:?} is not an image, returning default");
        match self.default_media(MediaType::Image).as_deref() {
            Some(Media::Image(img)) => Some(img.clone()),
            _ => None,
        }
    }

    /// Falls back to the default SVG if `media_id` is missing or is not an SVG, and to `None` if
    /// there is no default either.
    pub fn get_svg(&self, media_id: MediaId) -> Option<Arc<MediaSvg>> {
        if let Some(Media::Svg(media_svg)) = self.get(media_id, MediaType::Svg).as_deref() {
            return Some(media_svg.clone());
        }
        log::warn!("Media {media_id:?} is not an SVG, returning default");
        match self.default_media(MediaType::Svg).as_deref() {
            Some(Media::Svg(svg)) => Some(svg.clone()),
            _ => None,
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
    ///
    /// `None` if even the default is absent - which means there is nothing to draw, and every
    /// caller already had a path for that.
    pub fn get(&self, media_id: MediaId, media_type: MediaType) -> Option<Arc<Media>> {
        let resident = self.entries.read().get(&media_id).cloned();
        if let Some(media) = resident {
            if self.decoded_budget.load(Ordering::Relaxed) != 0 {
                self.touch(media_id);
            }
            return Some(media);
        }
        self.revive(media_id).or_else(|| self.default_media(media_type))
    }

    fn default_media(&self, media_type: MediaType) -> Option<Arc<Media>> {
        match media_type {
            MediaType::Svg => self.default_svg.clone(),
            MediaType::Image => self.default_image.clone(),
        }
    }

    /// Wait for a resource's bytes, asking for them first if nobody else has.
    ///
    /// This runs on the background thread [`MediaStore::request_media`] spawns, never on the
    /// layout thread, so waiting here costs a thread and nothing else. It does not fetch:
    /// the request goes to the [`MediaSource`], which puts it through the engine's fetcher
    /// with the policy, cache and observation that belong to it, and the bytes come back
    /// through the handoff. Classification is left to the decoder registry, which treats the
    /// content type as a hint only.
    fn fetch_resource(&self, src: &str) -> anyhow::Result<(Option<String>, Bytes)> {
        let url = Url::parse(src)?;
        let _t = gosub_shared::timing_guard!(gosub_shared::timing::Timing::NetFetchImage, src);

        // The source first, because announcing a fetch obliges someone to answer it: a claim
        // made with nothing behind it to do the fetching strands the entry, and every later
        // consumer of that URL then waits out the timeout for bytes nobody is bringing.
        let Some(source) = self.source.read().clone() else {
            anyhow::bail!("no media source is wired up, so {url} cannot be loaded");
        };
        // The source claims and asks in one step, and hands back the scope it used; waiting
        // under a scope this side worked out separately would race a page change.
        let scope = match source.acquire(src) {
            Acquired::Under(scope) => scope,
            Acquired::Later => return Err(NotYet.into()),
            Acquired::Unowned => {
                anyhow::bail!("no page is loaded yet, so {url} belongs to nothing and cannot be loaded")
            }
        };

        match gosub_shared::subresource::take(scope, src) {
            Some((content_type, body)) => Ok((content_type, Bytes::from(body))),
            None => anyhow::bail!("no bytes arrived for {url}"),
        }
    }
}

/// A load the source is still working on: "not yet" rather than "no".
#[derive(Debug)]
struct NotYet;

impl std::fmt::Display for NotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("still being fetched")
    }
}

impl std::error::Error for NotYet {}

/// Whether a load error says "not yet" rather than "no".
fn is_pending(e: &anyhow::Error) -> bool {
    e.downcast_ref::<NotYet>().is_some()
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
        // Percent-decode a plain (text) payload, e.g. `data:image/svg+xml,<svg ...>`.
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

/// Rasterize a `usvg` tree to a straight-alpha RGBA [`Image`] of `w`x`h` px (scaling the tree's
/// intrinsic size to fit). Returns `None` if the pixmap can't be allocated.
pub fn render_svg_tree_to_image(tree: &resvg::usvg::Tree, w: u32, h: u32) -> Option<Image> {
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
mod brokered_decoder_tests {
    use super::*;
    use gosub_interface::media_decoder::{DecodeError, RasterImage};

    /// Answers every decode with a 2x1 raster standing for a 4000x2000 image,
    /// the way the decoder process reports a downscaled one.
    #[derive(Debug)]
    struct Downscaling;

    impl ImageDecoder for Downscaling {
        fn decode(&self, _mime: Option<&str>, _bytes: &[u8]) -> Result<BrokeredDecode, DecodeError> {
            Ok(BrokeredDecode::Raster(RasterImage {
                width: 2,
                height: 1,
                intrinsic_width: 4000,
                intrinsic_height: 2000,
                rgba: bytes::Bytes::from(vec![255u8; 2 * 4]),
            }))
        }

        fn dimensions(&self, _mime: Option<&str>, _bytes: &[u8]) -> Result<(u32, u32), DecodeError> {
            Ok((4000, 2000))
        }
    }

    #[test]
    fn a_brokered_raster_lays_out_at_its_intrinsic_size() {
        let store = MediaStore::with_decoder(Some(Arc::new(Downscaling)));
        let id = store
            .load_media_from_data(MediaType::Image, b"any bytes: the fake decoder does not look")
            .expect("decodes");
        let image = store.get_image(id).expect("a raster");
        assert_eq!((image.image.width(), image.image.height()), (2, 1));
        assert_eq!(
            (image.image.intrinsic_width(), image.image.intrinsic_height()),
            (4000, 2000)
        );
    }

    #[test]
    fn inline_svg_stays_a_vector_with_a_decoder_installed() {
        let store = MediaStore::with_decoder(Some(Arc::new(Downscaling)));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"></svg>"#;
        let id = store.load_media_from_data(MediaType::Svg, svg).expect("decodes");
        assert!(
            store.get_svg(id).is_some(),
            "inline SVG must stay an SVG, not come back rasterized"
        );
    }
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

            let img = store.get_image(media_id).expect("decoded image should be in the store");
            assert_eq!(img.image.width(), 8, "{format:?} width");
            assert_eq!(img.image.height(), 4, "{format:?} height");
        }
    }

    /// Decoding must land under `decode.image`, and must not swallow any transfer time -
    /// this path only ever sees bytes that are already in hand.
    #[test]
    fn decode_is_recorded_under_decode_image() {
        // The timing table is process-global and tests run in parallel, so this asserts a
        // relative increase rather than an absolute count, and never calls reset_stats()
        // (which would wipe whatever a concurrent test is recording).
        let count_of = |ns: &str| {
            gosub_shared::timing::snapshot_stats()
                .iter()
                .find(|s| s.namespace == ns)
                .map_or(0, |s| s.count)
        };
        let before = count_of("decode.image");

        let store = MediaStore::new();
        let bytes = encode(ImageFormat::Png);
        let media_id = store
            .load_media_from_data(MediaType::Image, &bytes)
            .unwrap_or_else(|e| panic!("png failed to load: {e}"));
        assert!(!store.is_placeholder(media_id));

        assert!(
            count_of("decode.image") > before,
            "decoding did not record a decode.image sample"
        );
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
        let svg = store.get_svg(media_id).expect("decoded svg should be in the store");
        let size = svg.svg.tree.size();
        assert_eq!((size.width() as u32, size.height() as u32), (20, 10));
    }

    /// A source that records what it was asked for and answers with whatever it was given.
    #[derive(Debug)]
    struct FakeSource {
        asked: parking_lot::Mutex<Vec<String>>,
        answer: Option<Vec<u8>>,
    }

    /// The scope a test source fetches in. Which one does not matter, only that producer and
    /// consumer agree on it, the way a navigation and its document do.
    const PAGE: gosub_shared::subresource::Scope = 7;

    impl MediaSource for FakeSource {
        fn acquire(&self, url: &str) -> Acquired {
            if gosub_shared::subresource::claim(PAGE, url) {
                self.asked.lock().push(url.to_string());
                match &self.answer {
                    Some(bytes) => {
                        gosub_shared::subresource::complete(PAGE, url, Some("image/png".into()), bytes.clone())
                    }
                    None => gosub_shared::subresource::abandon(PAGE, url),
                }
            }
            Acquired::Under(PAGE)
        }
    }

    /// The resource handoff is process-wide, so these tests take turns with it: run in
    /// parallel they clear the store out from under each other.
    fn exclusively<T>(body: impl FnOnce() -> T) -> T {
        static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let _guard = LOCK.lock();
        gosub_shared::subresource::clear();
        body()
    }

    fn wired(answer: Option<Vec<u8>>) -> (Arc<MediaStore>, Arc<FakeSource>) {
        let store = Arc::new(MediaStore::new());
        let source = Arc::new(FakeSource {
            asked: parking_lot::Mutex::new(Vec::new()),
            answer,
        });
        store.set_source(source.clone());
        (store, source)
    }

    /// The store does not fetch: it names what it needs and takes what arrives.
    #[test]
    fn an_unclaimed_resource_is_asked_for_and_taken() {
        exclusively(|| {
            let (store, source) = wired(Some(encode(ImageFormat::Png)));

            let (content_type, body) = store
                .fetch_resource("https://example.test/unclaimed.png")
                .expect("bytes should arrive");

            assert_eq!(source.asked.lock().as_slice(), ["https://example.test/unclaimed.png"]);
            assert_eq!(content_type.as_deref(), Some("image/png"));
            assert!(!body.is_empty());
        });
    }

    /// A resource the document scan already claimed is on its way, and asking again would
    /// fetch it twice — which is the whole reason the handoff exists.
    #[test]
    fn a_resource_already_in_flight_is_waited_for_rather_than_asked_for() {
        exclusively(|| {
            let url = "https://example.test/claimed.png";
            gosub_shared::subresource::begin(PAGE, url);
            gosub_shared::subresource::complete(PAGE, url, Some("image/png".into()), encode(ImageFormat::Png));

            let (store, source) = wired(None);
            let (_, body) = store.fetch_resource(url).expect("the delivered bytes");

            assert!(source.asked.lock().is_empty(), "should not have asked for it again");
            assert!(!body.is_empty());
        });
    }

    /// A store nobody wired up has nowhere to ask, and says so rather than reaching for a
    /// network of its own — which is what it used to do.
    #[test]
    fn a_store_with_no_source_loads_nothing() {
        exclusively(|| {
            let store = MediaStore::new();
            let err = store
                .fetch_resource("https://example.test/nosource.png")
                .expect_err("should not load");
            assert!(err.to_string().contains("no media source"), "got: {err}");
        });
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
    fn synchronous_loads_decode_on_first_use_not_on_load() {
        let store = Arc::new(MediaStore::new());
        store.set_synchronous_fetch(true);
        let before = store.resident_bytes();
        let MediaRequest::Ready(id) = store.request_media(&data_uri(64, 32, 7)) else {
            panic!("synchronous load should be ready");
        };
        // Layout gets the size; nothing was decoded for it.
        assert_eq!(store.image_intrinsic_size(id), Some((64, 32)));
        assert_eq!(store.resident_bytes(), before);
        assert!(!store.entries.read().contains_key(&id));
        // Paint asks for pixels: decoded now, at the real size.
        let image = store.get_image(id).expect("decoded on use");
        assert_eq!((image.image.width(), image.image.height()), (64, 32));
        assert!(store.entries.read().contains_key(&id));
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
        let back = store.get(ids[0], MediaType::Image);
        let Some(Media::Image(back)) = back.as_deref() else {
            panic!("expected an image");
        };
        assert_eq!((back.image.width(), back.image.height()), (100, 100));
        assert_eq!(back.image.as_raw()[0], 0);
        assert!(store.entries.read().contains_key(&ids[0]));
        assert!(resident(&store) <= 2);
        assert!(matches!(store.request_media(&data_uri(100, 100, 0)), MediaRequest::Ready(id) if id == ids[0]));
    }
}
