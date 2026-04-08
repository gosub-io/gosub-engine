use anyhow::anyhow;
use fontique::{Attributes, Collection, GenericFamily, QueryFamily, QueryStatus, SourceCache};
use parley::Font;

/// A simple font manager that uses Fontique to manage and resolve fonts.
pub struct FontManager {
    collection: Collection,
    cache: SourceCache,
}

impl FontManager {
    pub fn new() -> Self {
        Self {
            collection: Collection::new(Default::default()),
            cache: SourceCache::new_shared(),
        }
    }

    /// Resolve a preferred family name; falls back to UI Sans → SansSerif.
    pub fn resolve_ui_font(&mut self, prefer: Option<&str>, attrs: Attributes) -> anyhow::Result<(Font, String)> {
        let mut col_clone = self.collection.clone();

        let mut q = self.collection.query(&mut self.cache);

        // Build a reasonable fallback stack:
        let mut families: Vec<QueryFamily> = Vec::new();
        if let Some(name) = prefer {
            families.push(QueryFamily::Named(name));
        }
        families.push(GenericFamily::UiSansSerif.into());
        families.push(GenericFamily::SansSerif.into());

        q.set_families(families);
        q.set_attributes(attrs);

        let mut chosen: Option<(Font, String)> = None;
        q.matches_with(|cand| {
            let vello_font = Font::new(cand.blob.clone(), cand.index);

            let (fam_id, _) = cand.family;
            let fam_info = col_clone.family(fam_id).expect("family id invalid");

            chosen = Some((vello_font, fam_info.name().to_string()));
            QueryStatus::Stop
        });

        if chosen.is_some() {
            return Ok(chosen.unwrap());
        }

        Err(anyhow!("Failed to resolve font"))
    }
}
