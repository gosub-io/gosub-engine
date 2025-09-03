use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, OnceLock, RwLock};
use crate::common::texture::{Texture, TextureId};

pub static TEXTURE_STORE: OnceLock<RwLock<TextureStore>> = OnceLock::new();

pub fn get_texture_store() -> &'static RwLock<TextureStore> {
    TEXTURE_STORE.get_or_init(|| RwLock::new(TextureStore::new()))
}


/// Texture store stores all the textures. It can remove textures if needed (LRU / memory constraints for instance).
pub struct TextureStore {
    textures: HashMap<TextureId, Arc<Texture>>,
    next_id: RwLock<TextureId>,
}

impl TextureStore {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            next_id: RwLock::new(TextureId::new(0)),
        }
    }

    pub fn add(&mut self, width: usize, height: usize, data: Vec<u8>) -> TextureId {
        let texture = Texture {
            id: self.next_id(),
            width,
            height,
            data,
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
        let mut nid = self.next_id.write().expect("Failed to lock next texture ID");
        let id = *nid;
        *nid += 1;
        id
    }

    pub fn save_to_disk(&self, texture_id: TextureId) -> Result<(), String> {
        if let Some(texture) = self.get(texture_id) {

            let img = match image::RgbaImage::from_raw(
                texture.width as u32,
                texture.height as u32,
                texture.data.clone(),
            ) {
                Some(img) => img,
                None => return Err(format!("Failed to create image from texture data")),
            };

            let mut png_data = Vec::new();
            let mut cursor = Cursor::new(&mut png_data);
            img.write_to(&mut cursor, image::ImageFormat::Png).map_err(|e| format!("Failed to write image to PNG: {}", e))?;
            std::fs::write(format!("texture-{}.png", texture_id.as_u64()), png_data).map_err(|e| format!("Failed to save texture to disk: {}", e))?;
            Ok(())
        } else {
            Err(format!("Texture with ID {} not found", texture_id))
        }
    }
}
