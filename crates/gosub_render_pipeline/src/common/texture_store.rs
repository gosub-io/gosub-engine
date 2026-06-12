use crate::common::texture::{Texture, TextureId};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Texture store for a single rasterizer instance. Create one per render context; do not share
/// globally between tabs.
pub struct TextureStore {
    textures: HashMap<TextureId, Arc<Texture>>,
    next_id: RwLock<TextureId>,
}

impl Default for TextureStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TextureStore {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            next_id: RwLock::new(TextureId::new(0)),
        }
    }

    pub fn add(
        &mut self,
        width: usize,
        height: usize,
        data: Vec<u8>,
        format: crate::render::backend::PixelFormat,
    ) -> TextureId {
        let texture = Texture {
            id: self.next_id(),
            width,
            height,
            data: std::sync::Arc::new(data),
            format,
        };

        let id = texture.id;
        self.textures.insert(texture.id, Arc::new(texture));

        id
    }

    #[allow(unused)]
    pub fn has(&self, texture_id: TextureId) -> bool {
        self.textures.contains_key(&texture_id)
    }

    pub fn get(&self, texture_id: TextureId) -> Option<Arc<Texture>> {
        self.textures.get(&texture_id).cloned()
    }

    #[allow(unused)]
    pub fn remove(&mut self, texture_id: TextureId) {
        self.textures.remove(&texture_id);
    }

    fn next_id(&self) -> TextureId {
        let mut nid = self.next_id.write();
        let id = *nid;
        *nid += 1;
        id
    }
}
