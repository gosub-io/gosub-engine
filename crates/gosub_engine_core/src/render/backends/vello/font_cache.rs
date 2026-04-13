use parley::Font;
use std::collections::HashMap;

/// A simple font cache that maps font family names to loaded fonts.
pub struct FontCache {
    fonts: HashMap<String, Font>,
    resolved_names: HashMap<String, String>,
}

impl FontCache {
    /// Create a new, empty font cache.
    pub fn new() -> Self {
        Self {
            fonts: HashMap::new(),
            resolved_names: HashMap::new(),
        }
    }

    /// Resolve a preferred family name; falls back to UI Sans → SansSerif.
    pub fn fetch(&mut self, name: &str) -> Option<(&Font, String)> {
        match self.fonts.get(name) {
            Some(font) => Some((font, name.to_string())),
            None => {
                if let Some(resolved_name) = self.resolved_names.get(name) {
                    if let Some(font) = self.fonts.get(resolved_name) {
                        return Some((font, resolved_name.clone()));
                    }
                }

                let fallback_name = match name {
                    "UI Sans" => "SansSerif",
                    _ => "SansSerif",
                };

                if let Some(font) = self.fonts.get(fallback_name) {
                    self.resolved_names.insert(name.to_string(), fallback_name.to_string());
                    return Some((font, fallback_name.to_string()));
                }

                None
            }
        }
    }

    pub fn insert(&mut self, name: &str, resolved_name: &str, font: Font) {
        self.fonts.insert(name.to_string(), font);
        self.resolved_names.insert(name.to_string(), resolved_name.to_string());
    }

    #[allow(unused)]
    pub fn clear(&mut self) {
        self.fonts.clear();
        self.resolved_names.clear();
    }

    #[allow(unused)]
    pub fn remove(&mut self, name: &str) {
        self.fonts.remove(name);
        self.resolved_names.remove(name);
    }
}
