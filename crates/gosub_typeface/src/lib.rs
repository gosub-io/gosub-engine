// use std::sync::{Arc, Mutex};
//
// use lazy_static::lazy_static;
// use rust_fontconfig::{FcFontCache, FcPattern};
//
// pub const DEFAULT_FS: f32 = 12.0; //TODO: these need to be moved to somewhere and made configurable
// pub const DEFAULT_LH: f32 = 1.2;
//
// #[derive(Clone, PartialEq, Debug)]
// pub struct SharedFont {
//     pub data: Arc<Vec<u8>>,
//     pub ty: FontType,
// }
//
// impl SharedFont {
//     pub fn new(data: Arc<Vec<u8>>) -> Self {
//         Self {
//             data,
//             ty: FontType::Unknown,
//         }
//     }
//
//     pub fn unknown(data: Arc<Vec<u8>>) -> Self {
//         Self {
//             data,
//             ty: FontType::Unknown,
//         }
//     }
// }
//
// #[derive(Clone, PartialEq, Debug)]
// pub struct Font {
//     pub data: Vec<u8>,
//     pub ty: FontType,
// }
//
// #[derive(Clone, PartialEq, Debug)]
// pub enum FontType {
//     TrueType,
//     OpenType,
//     Woff,
//     Woff2,
//     Svg,
//     Unknown,
//     //TODO: add others (maybe)
// }
//
// #[cfg(target_arch = "wasm32")]
// #[derive(Default, Clone, Debug, PartialEq, Eq)]
// pub struct FcPattern {
//     name: Option<String>,
// }
//
// #[cfg(not(target_arch = "wasm32"))]
// lazy_static! {
//     pub static ref FONT_PATH_CACHE: FcFontCache = FcFontCache::build();
// }
//
// const ROBOTO_REGULAR: &[u8] = include_bytes!("../../../resources/fonts/Roboto-Regular.ttf");
//
// lazy_static! {
//     pub static ref BACKUP_FONT: SharedFont = SharedFont {
//         data: Arc::new(ROBOTO_REGULAR.to_vec()),
//         ty: FontType::TrueType,
//     };
// }
//
// lazy_static! {
//     pub static ref FONT_RENDERER_CACHE: Mutex<FontRendererCache> = {
//         let backup = TextRenderer {
//             pattern: FcPattern {
//                 name: Some("Roboto".to_string()),
//                 ..Default::default()
//             },
//             font: BACKUP_FONT.clone(),
//             sizing: Vec::new(),
//         };
//
//         Mutex::new(FontRendererCache::new(backup))
//     };
// }
//
// pub struct FontRendererCache {
//     renderers: Vec<TextRenderer>,
//     pub backup: TextRenderer,
// }
//
// enum Index {
//     Some(usize),
//     Backup,
// }
//
// impl Index {
//     fn is_backup(&self) -> bool {
//         matches!(self, Self::Backup)
//     }
// }
//
// impl From<Option<usize>> for Index {
//     fn from(index: Option<usize>) -> Self {
//         match index {
//             Some(index) => Self::Some(index),
//             None => Self::Backup,
//         }
//     }
// }
//
// #[allow(dead_code)]
// enum IndexNoBackup {
//     None,
//     Some(usize),
//     Insert(String),
// }
//
// impl IndexNoBackup {
//     fn is_none(&self) -> bool {
//         matches!(self, Self::None)
//     }
// }
//
// impl From<Option<usize>> for IndexNoBackup {
//     fn from(index: Option<usize>) -> Self {
//         match index {
//             Some(index) => Self::Some(index),
//             None => Self::None,
//         }
//     }
// }
//
// impl FontRendererCache {
//     fn new(backup: TextRenderer) -> Self {
//         Self {
//             renderers: Vec::new(),
//             backup,
//         }
//     }
//
//     fn query_no_backup(&mut self, pattern: FcPattern) -> IndexNoBackup {
//         let index: IndexNoBackup = self
//             .renderers
//             .iter()
//             .position(|r| r.pattern == pattern)
//             .into();
//
//         if index.is_none() {
//             #[cfg(not(target_arch = "wasm32"))]
//             {
//                 let Some(font_path) = FONT_PATH_CACHE.query(&pattern) else {
//                     return IndexNoBackup::None;
//                 };
//
//                 return IndexNoBackup::Insert(font_path.path.clone());
//             }
//             #[cfg(target_arch = "wasm32")]
//             return IndexNoBackup::None;
//         }
//
//         index
//     }
//
//     fn query_font_no_backup(&mut self, pattern: FcPattern) -> Option<SharedFont> {
//         let font = self.query_no_backup(pattern);
//
//         match font {
//             IndexNoBackup::Some(index) => Some(SharedFont::clone(&self.renderers[index].font)),
//             IndexNoBackup::Insert(path) => {
//                 let font_bytes = std::fs::read(&path).expect("Failed to read font file");
//
//                 let font = SharedFont::unknown(Arc::new(font_bytes));
//
//                 let r = TextRenderer {
//                     pattern: FcPattern {
//                         name: Some(path),
//                         ..Default::default()
//                     },
//                     font: SharedFont::clone(&font),
//                     sizing: Vec::new(),
//                 };
//
//                 self.renderers.push(r);
//
//                 Some(font)
//             }
//             IndexNoBackup::None => None,
//         }
//     }
//
//     pub fn query(&mut self, pattern: FcPattern) -> &mut TextRenderer {
//         if self.backup.pattern == pattern {
//             return &mut self.backup;
//         }
//
//         // we need to do this with an index value because of https://github.com/rust-lang/rust/issues/21906
//         #[allow(unused_mut)]
//         let mut index: Index = self
//             .renderers
//             .iter()
//             .position(|r| r.pattern == pattern)
//             .into();
//
//         if index.is_backup() {
//             #[cfg(not(target_arch = "wasm32"))]
//             {
//                 let Some(font_path) = FONT_PATH_CACHE.query(&pattern) else {
//                     return &mut self.backup;
//                 };
//
//                 let Ok(font_bytes) = std::fs::read(&font_path.path) else {
//                     return &mut self.backup;
//                 };
//
//                 let font = SharedFont::new(Arc::new(font_bytes));
//
//                 let r = TextRenderer {
//                     pattern,
//                     font,
//                     sizing: Vec::new(),
//                 };
//
//                 self.renderers.push(r);
//                 index = Index::Some(self.renderers.len() - 1);
//             }
//             #[cfg(target_arch = "wasm32")]
//             return &mut self.backup;
//         }
//
//         match index {
//             Index::Some(index) => &mut self.renderers[index],
//             Index::Backup => &mut self.backup,
//         }
//     }
//
//     pub fn query_ff(&mut self, font_family: Vec<String>) -> &mut TextRenderer {
//         let mut renderer = IndexNoBackup::None;
//         for f in font_family {
//             let pattern = FcPattern {
//                 name: Some(f),
//                 ..Default::default()
//             };
//
//             let rend = self.query_no_backup(pattern);
//
//             match rend {
//                 IndexNoBackup::Some(index) => {
//                     return &mut self.renderers[index];
//                 }
//                 IndexNoBackup::Insert(path) => {
//                     renderer = IndexNoBackup::Insert(path);
//                 }
//                 IndexNoBackup::None => {}
//             }
//         }
//
//         match renderer {
//             IndexNoBackup::Some(index) => &mut self.renderers[index], //unreachable, but we handle it just in case
//             IndexNoBackup::Insert(path) => {
//                 let font_bytes = std::fs::read(&path).expect("Failed to read font file");
//                 let font = SharedFont::unknown(Arc::new(font_bytes));
//
//                 let r = TextRenderer {
//                     pattern: FcPattern {
//                         name: Some(path),
//                         ..Default::default()
//                     },
//                     font,
//                     sizing: Vec::new(),
//                 };
//
//                 let idx = self.renderers.len();
//                 self.renderers.push(r);
//                 &mut self.renderers[idx]
//             }
//             IndexNoBackup::None => &mut self.backup,
//         }
//     }
//
//     pub fn query_all_shared(&mut self, font_family: Vec<String>) -> Vec<Arc<Vec<u8>>> {
//         let mut fonts = Vec::with_capacity(font_family.len());
//
//         for f in font_family {
//             let pattern = FcPattern {
//                 name: Some(f),
//                 ..Default::default()
//             };
//
//             let font = self.query_font_no_backup(pattern);
//
//             if let Some(font) = font {
//                 fonts.push(font.data);
//             }
//         }
//
//         fonts
//     }
// }
//
// #[derive(Clone)]
// pub struct TextRenderer {
//     pub pattern: FcPattern,
//     pub font: SharedFont,
//     pub sizing: Vec<FontSizing>,
// }
//
// #[derive(Clone)]
// pub struct FontSizing {
//     pub font_size: f32,
//     pub line_height: f32,
// }

pub mod font;

pub const ROBOTO_FONT: &[u8] = include_bytes!("../../../resources/fonts/Roboto-Regular.ttf");
