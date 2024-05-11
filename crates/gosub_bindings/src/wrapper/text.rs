use gosub_rendering::render_tree::text::TextNode;
use std::ffi::c_char;
use std::ffi::CString;

/// This is a C-friendly wrapper around `gosub_render_utils::rendertree::text::TextNode`
/// that converts Rust Strings to owned pointers to pass to the C API.
#[repr(C)]
pub struct CTextNode {
    pub value: *mut c_char,
    pub font: *mut c_char,
    pub font_size: f64,
    pub is_bold: bool,
}

impl From<&TextNode> for CTextNode {
    fn from(text_node: &TextNode) -> Self {
        Self {
            value: CString::new(text_node.value.clone().into_bytes())
                .expect("Failed to allocate memory for text node value in CTextNode")
                .into_raw(),
            font: CString::new(text_node.font.clone().into_bytes())
                .expect("Failed to allocate memory for text node font in CTextNode")
                .into_raw(),
            font_size: text_node.font_size,
            is_bold: text_node.is_bold,
        }
    }
}
