use crate::font_manager::font_info::FontInfo;
use gosub_interface::font::FontStyle;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

lazy_static! {
    pub static ref FONT_CACHE: Arc<Mutex<MemoryCache>> = Arc::new(Mutex::new(MemoryCache::new()));
}

#[allow(unused)]
pub(crate) trait Cache {
    fn get(&self, family: &str, style: FontStyle) -> Option<FontInfo>;
    fn set(&mut self, family: &str, style: FontStyle, font_info: &FontInfo);
    fn clear(&mut self);
    fn remove(&mut self, family: &str, style: FontStyle);
}

// Some kind of caching strategy for font_info stuff
pub struct MemoryCache {
    cache: HashMap<String, FontInfo>,
}

impl MemoryCache {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }
}

impl Cache for MemoryCache {
    fn get(&self, family: &str, style: FontStyle) -> Option<FontInfo> {
        let key = format!("{}-{}", family, style);
        self.cache.get(&key).cloned()
    }

    fn set(&mut self, family: &str, style: FontStyle, font_info: &FontInfo) {
        let key = format!("{}-{}", family, style);
        self.cache.insert(key, font_info.clone());
    }

    fn clear(&mut self) {
        self.cache.clear();
    }

    fn remove(&mut self, family: &str, style: FontStyle) {
        let key = format!("{}-{}", family, style);
        self.cache.remove(&key);
    }
}
