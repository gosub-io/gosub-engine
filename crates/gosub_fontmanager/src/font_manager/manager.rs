use crate::font_manager::font_info::FontInfo;
use anyhow::anyhow;
use font_kit::handle::Handle;
use gosub_interface::font::FontManager as TFontManager;
use gosub_interface::font::FontStyle;
use log::error;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[allow(dead_code)]
pub const LOG_TARGET: &str = "font-manager";

thread_local! {
    static FONT_MANAGER: Arc<RwLock<FontManager>> = Arc::new(RwLock::new(FontManager::new()));
}

pub struct FontManager {
    /// Vec of all font-info structures found
    available_fonts: Vec<FontInfo>,
}

impl Default for FontManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FontManager {
    pub fn new() -> Self {
        let source = font_kit::source::SystemSource::new();
        let handles = source.all_fonts().unwrap();

        let mut seen_paths: HashSet<PathBuf> = HashSet::new();

        let mut font_info_list = Vec::new();
        for handle in &handles {
            if let Ok(info) = handle_to_info(&mut seen_paths, handle) {
                font_info_list.push(info)
            }
        }

        font_info_list.sort_by_key(|fi| fi.family.clone());

        Self {
            available_fonts: font_info_list,
        }
    }

    /// Returns all available fonts for given source-type
    pub fn available_fonts(&self) -> &Vec<FontInfo> {
        &self.available_fonts
    }

    pub fn find(&self, families: &[&str], style: FontStyle) -> Option<FontInfo> {
        for &fam in families {
            for fi in self.available_fonts() {
                if fi.family.eq_ignore_ascii_case(fam) && fi.style == style {
                    return Some(fi.clone());
                }
            }
        }

        None
    }
}

impl TFontManager for FontManager {
    type FontInfo = FontInfo;

    fn instance() -> Arc<RwLock<Self>> {
        FONT_MANAGER.with(|f| f.clone())
    }

    fn find_font(&self, families: &[&str], style: FontStyle) -> Option<Self::FontInfo> {
        for &fam in families {
            for fi in &self.available_fonts {
                if fi.family.eq_ignore_ascii_case(fam) && fi.style == style {
                    return Some(fi.clone());
                }
            }
        }
        None
    }
}

fn handle_to_info(seen_paths: &mut HashSet<PathBuf>, handle: &Handle) -> Result<FontInfo, anyhow::Error> {
    let font = handle.load().unwrap();

    let family = font.family_name();
    let props = font.properties();

    let style = match props.style {
        font_kit::properties::Style::Normal => FontStyle::Normal,
        font_kit::properties::Style::Italic => FontStyle::Italic,
        font_kit::properties::Style::Oblique => FontStyle::Oblique,
    };

    let Handle::Path { ref path, font_index } = handle else {
        error!(target: LOG_TARGET, "Expected a path handle. Got: {:?}", handle);
        return Err(anyhow!("Expected a path handle"));
    };

    // Check if the path is symlinked
    let resolved_path = resolve_symlink(path.to_path_buf());
    if seen_paths.contains(&resolved_path) {
        return Err(anyhow!("Path already seen"));
    }
    seen_paths.insert(resolved_path.clone());

    Ok(FontInfo {
        family,
        style,
        weight: props.weight.0 as i32,
        stretch: props.stretch.0,
        monospaced: font.is_monospace(),
        path: Some(resolved_path.clone()),
        index: Some(*font_index as i32),
    })
}

/// Resolves a symlinked path
fn resolve_symlink(path: PathBuf) -> PathBuf {
    let mut resolved_path = path.clone();

    while let Ok(target) = std::fs::read_link(&resolved_path) {
        resolved_path = if target.is_relative() {
            path.parent().unwrap_or(Path::new("/")).join(target)
        } else {
            target
        };
    }

    resolved_path
}
