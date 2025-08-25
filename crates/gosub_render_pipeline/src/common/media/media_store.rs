use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use bytes::Bytes;
use file_type::FileType;
use reqwest::header::HeaderValue;
use resvg::usvg;
use crate::common::hash::{hash_from_data, hash_from_string, Sha256Hash};
use crate::common::media::{Media, MediaId, MediaImage, MediaSvg, MediaType};
use crate::common::media::Svg;

const DEFAULT_SVG_ID: MediaId = MediaId::new(0);
const DEFAULT_IMAGE_ID: MediaId = MediaId::new(1);
const FIRST_FREE_IMAGE_ID: u64 = 100;

const DEFAULT_SVG_DATA: &[u8] = include_bytes!("../../../resources/not-found.svg");
const DEFAULT_IMAGE_DATA: &[u8] = include_bytes!("../../../resources/default-image.png");

/// Media store is global
pub static MEDIA_STORE: OnceLock<RwLock<MediaStore>> = OnceLock::new();

pub fn get_media_store() -> &'static RwLock<MediaStore> {
    MEDIA_STORE.get_or_init(|| RwLock::new(MediaStore::new()))
}

/// Media store keeps all the loaded media in memory so it can be referenced by its MediaID
pub struct MediaStore {
    /// List of all media
    pub entries: RwLock<HashMap<MediaId, Arc<Media>>>,
    /// List of all images by hash(src)
    pub cache: RwLock<HashMap<Sha256Hash, MediaId>>,
    /// Next media ID
    next_id: RwLock<MediaId>,
}


impl MediaStore {
    pub fn new() -> MediaStore {
        let store = MediaStore {
            entries: RwLock::new(HashMap::new()),
            cache: RwLock::new(HashMap::new()),
            next_id: RwLock::new(MediaId::new(FIRST_FREE_IMAGE_ID)),
        };

        // Add "default svg" to the store.
        let default_svg_tree = usvg::Tree::from_data(&DEFAULT_SVG_DATA, &usvg::Options::default()).expect("Failed to load default svg");
        let mut entries = store.entries.write().expect("Failed to lock images");
        let media = Media::svg("gosub://default/svg", Svg::new(default_svg_tree));
        entries.insert(DEFAULT_SVG_ID, Arc::new(media));
        drop(entries);

        // Add "default image" to the store.
        let default_image = image::load_from_memory(&DEFAULT_IMAGE_DATA).expect("Failed to load default image").to_rgba8();
        let mut entries = store.entries.write().expect("Failed to lock images");
        let media = Media::image("gosub://default/image", default_image);
        entries.insert(DEFAULT_IMAGE_ID, Arc::new(media));
        drop(entries);

        store
    }

    /// Load the given media from src into the media store, and return the media ID. Will also store the media(id) in cache
    /// so the next call with the same src will return the same media ID without reloading.
    pub fn load_media(&self, src: &str) -> anyhow::Result<MediaId> {
        // Check if the media from src is already loaded into the cache. If so, return that
        let h = hash_from_string(src);
        let cache = self.cache.read().expect("Failed to lock cache");
        if let Some(media_id) = cache.get(&h) {
            println!("Loading cached media from path: {}", src);
            return Ok(*media_id);
        }
        drop(cache);

        let result = self.load_media_from_source(src);

        // Store it in cache
        if let Ok(media_id) = result{
            let mut cache = self.cache.write().expect("Failed to lock cache");
            cache.insert(h, media_id);
        }

        result
    }

    pub fn load_media_from_data(&self, media_type: MediaType, data: &[u8]) -> anyhow::Result<MediaId> {
        let h = hash_from_data(data);
        let cache = self.cache.read().expect("Failed to lock cache");
        if let Some(media_id) = cache.get(&h) {
            println!("Loading cached media from data");
            return Ok(*media_id);
        }
        drop(cache);

        let media_id = match media_type {
            MediaType::Svg => {
                let svg_tree = match usvg::Tree::from_data(data, &usvg::Options::default()) {
                    Ok(tree) => tree,
                    Err(_) => {
                        return Err(anyhow::anyhow!("Failed to parse SVG data"));
                    }
                };

                let media = Media::svg("gosub://data/svg", Svg::new(svg_tree));
                let media_id = *self.next_id.read().expect("Failed to lock next media ID");
                *self.next_id.write().expect("Failed to lock next media ID") += 1;

                let mut entries = self.entries.write().expect("Failed to lock entries");
                entries.insert(media_id, Arc::new(media));

                let mut cache = self.cache.write().expect("Failed to lock cache");
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
                let media_id = *self.next_id.read().expect("Failed to lock next media ID");
                *self.next_id.write().expect("Failed to lock next media ID") += 1;

                let mut entries = self.entries.write().expect("Failed to lock entries");
                entries.insert(media_id, Arc::new(media));

                let mut cache = self.cache.write().expect("Failed to lock cache");
                cache.insert(h, media_id);

                media_id
            }
        };

        let mut cache = self.cache.write().expect("Failed to lock cache");
        cache.insert(h, media_id);

        Ok(media_id)
    }

    fn load_media_from_source(&self, src: &str) -> anyhow::Result<MediaId> {
        println!("Loading non-cached media from path: {}", src);
        let Ok((media_type, raw_data)) = self.fetch_resource(src) else {
            anyhow::bail!("Failed to fetch resource");
        };

        let media = match media_type {
            MediaType::Svg => {
                let svg_tree = match usvg::Tree::from_data(&raw_data, &usvg::Options::default()) {
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

        let media_id = *self.next_id.read().expect("Failed to lock next media ID");
        *self.next_id.write().expect("Failed to lock next media ID") += 1;

        let mut entries = self.entries.write().expect("Failed to lock entries");
        entries.insert(media_id, Arc::new(media));

        Ok(media_id)
    }

    /// Returns a media image. If the media is not an image or does not exist, it will return the default media image id
    pub fn get_image(&self, media_id: MediaId) -> Arc<MediaImage> {
        let media = self.get(media_id, MediaType::Image);
        match &*media {
            Media::Image(media_image) => media_image.clone(),
            _ => unreachable!("Media is not an image"),
        }
    }

    /// Returns a media svg. If the media is not an svg or does not exist, it will return the default media svg id
    pub fn get_svg(&self, media_id: MediaId) -> Arc<MediaSvg> {
        let media = self.get(media_id, MediaType::Svg);
        match &*media {
            Media::Svg(media_svg) => media_svg.clone(),
            _ => unreachable!("Media is not an image"),
        }
    }

    pub fn update_svg(&self, media_id: MediaId, media: Arc<Media>) {
        let mut entries = self.entries.write().expect("Failed to lock images");
        entries.insert(media_id, media);
    }

    /// Returns a media resource. If the media does not exist, it will return the default media resource as specified by the media_type
    pub fn get(&self, media_id: MediaId, media_type: MediaType) -> Arc<Media> {
        let entries = self.entries.read().expect("Failed to lock images");

        match entries.get(&media_id) {
            Some(media) => media.clone(),
            None => self.default_media(media_type),
        }
    }

    /// Returns the default media resource for the given media type
    fn default_media(&self, media_type: MediaType) -> Arc<Media> {
        let entries = self.entries.read().expect("Failed to lock images");

        match media_type {
            MediaType::Svg => entries.get(&DEFAULT_SVG_ID).expect("Failed to get default svg").clone(),
            MediaType::Image => entries.get(&DEFAULT_IMAGE_ID).expect("Failed to get default image").clone(),
        }
    }

    /// Fetch resource from the web (or local file system, depending on the src) and returns the media type and raw
    /// bytes. This is blocking.
    fn fetch_resource(&self, src: &str) -> anyhow::Result<(MediaType, bytes::Bytes)> {
        let result = reqwest::blocking::get(src);
        let Ok(response) = result else {
            anyhow::bail!("Failed to fetch resource");
        };

        if !response.status().is_success() {
            anyhow::bail!("Incorrect http status code returned");
        }

        // Detect through content type
        let detected_content_type = detect_content_type(
            response.headers().get("content-type").unwrap_or(&HeaderValue::from_static("")).to_str().unwrap_or(""),
        );


        // Detect through content bytes
        let raw_bytes = response.bytes().unwrap_or(Bytes::new());
        let detected_file_type = detect_file_type(&raw_bytes);

        if detected_content_type.is_none() && detected_file_type.is_none() {
            anyhow::bail!("Failed to detect media type");
        }

        if detected_content_type == detected_file_type {
            return Ok((detected_content_type.unwrap(), raw_bytes));
        }

        // Seems that content type and file binaries are not matching. We will trust the file binary
        // over the content type.
        if detected_file_type.is_none() {
            Ok((detected_content_type.unwrap(), raw_bytes))
        } else {
            // Seems we cannot detect the file type from the binary. We will trust the content type
            Ok((detected_file_type.unwrap(), raw_bytes))
        }
    }
}


fn detect_file_type(data: &Bytes) -> Option<MediaType> {
    let ft = FileType::from_bytes(data);
    ft_detect(ft)
}

fn detect_content_type(content_type: &str) -> Option<MediaType> {
    let ft = FileType::from_media_type(content_type);
    if ft.is_empty() {
        return None;
    }
    ft_detect(ft[0])
}

fn ft_detect(ft: &FileType) -> Option<MediaType> {
    for &mt in ft.media_types().iter() {
        if mt == "image/svg+xml" {
            return Some(MediaType::Svg);
        }
        if mt.starts_with("image/") {
            return Some(MediaType::Image);
        }
    }

    None
}
