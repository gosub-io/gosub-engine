use std::collections::HashMap;
use gosub_render_backend::{ImageBuffer, ImageCacheEntry, ImgCache, RenderBackend, SizeU32};




#[derive(Debug)]
pub struct ImageCache<B: RenderBackend> {
    cache: HashMap<String, Entry<B>>,
}


impl<B: RenderBackend> ImgCache<B> for ImageCache<B> {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }
    fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: HashMap::with_capacity(capacity),
        }
    }

    fn add(&mut self, url: String, img: ImageBuffer<B>, size: Option<SizeU32>) {
        self.cache.insert(url, Entry::new(img, size));
    }

    fn add_pending(&mut self, url: String) {
        self.cache.insert(url, Entry::Pending);
    }

    fn get(&self, url: &str) -> ImageCacheEntry<B> {
        match self.cache.get(url) {
            Some(Entry::Image(img)) => ImageCacheEntry::Image(img),
            Some(Entry::SizedImg(_, img)) => ImageCacheEntry::Image(img),
            Some(Entry::Pending) => ImageCacheEntry::Pending,
            None => ImageCacheEntry::None,
        }
    }
}

#[derive(Debug)]
enum Entry<B: RenderBackend> {
    Pending,
    Image(ImageBuffer<B>),
    #[allow(unused)]
    SizedImg(SizeU32, ImageBuffer<B>),
}


impl<B: RenderBackend> Entry<B> {
    fn new(img: ImageBuffer<B>, size: Option<SizeU32>) -> Self {
        if let Some(size)  = size {
            Self::SizedImg(size, img)
        } else {
            Self::Image(img)
        }
    }
}