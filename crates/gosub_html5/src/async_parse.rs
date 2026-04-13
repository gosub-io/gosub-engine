//! Async HTML stream parsing with sub-resource discovery.
//!
//! Provides [`parse_html_stream`] which reads bytes asynchronously, parses them with
//! the real HTML5 parser, walks the DOM for sub-resource hints, and returns the real
//! [`Document`](gosub_interface::document::Document).

use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::{Document, DocumentBuilder};
use gosub_interface::node::{ElementDataType, Node, NodeData, NodeId, TextDataType};
use gosub_stream::byte_stream::{ByteStream, Encoding};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use url::Url;

/// A hint to the engine that a sub-resource should be fetched.
#[derive(Debug, Clone)]
pub struct ResourceHint {
    /// Absolute URL of the resource.
    pub url: Url,
    /// What kind of resource it is.
    pub kind: HintKind,
    /// Suggested fetch priority.
    pub priority: HintPriority,
    /// The integrity attribute value, if present.
    pub integrity: Option<String>,
    /// Whether this is a cross-origin request.
    pub cross_origin: bool,
}

/// The kind of sub-resource discovered.
#[derive(Debug, Clone, PartialEq)]
pub enum HintKind {
    Stylesheet,
    Script { defer: bool, async_load: bool },
    Image,
    Font,
}

/// Priority hint for a sub-resource.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HintPriority {
    High,
    Normal,
    Low,
}

/// Configuration for the async HTML stream parser.
#[derive(Debug, Clone)]
pub struct Html5ParseConfig {
    /// Maximum bytes to buffer from the stream.
    pub max_bytes: usize,
}

impl Default for Html5ParseConfig {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024,
        } // 4 MiB
    }
}

/// Error type for async stream parsing.
#[derive(thiserror::Error, Debug)]
pub enum ParseStreamError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Cancelled")]
    Cancelled,
}

/// Parse an HTML stream, discover sub-resources, and return the real DOM document.
///
/// - `base_url`: used as the document URL and to resolve relative URLs in resource hints.
/// - `reader`: async byte stream of HTML content.
/// - `cancel`: cancellation token; returns `Err(ParseStreamError::Cancelled)` if cancelled.
/// - `config`: parsing configuration.
/// - `on_discover`: called for each sub-resource hint found in the document.
pub async fn parse_html_stream<C, R, F>(
    base_url: Url,
    mut reader: R,
    cancel: CancellationToken,
    config: Html5ParseConfig,
    mut on_discover: F,
) -> Result<C::Document, ParseStreamError>
where
    C: HasDocument,
    R: AsyncRead + Unpin + Send + 'static,
    F: FnMut(ResourceHint),
{
    // Read bytes into a bounded buffer with cancellation support
    let mut buf = Vec::with_capacity(32 * 1024);
    let mut tmp = [0u8; 16 * 1024];

    loop {
        if cancel.is_cancelled() {
            return Err(ParseStreamError::Cancelled);
        }

        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        let remaining = config.max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }

        if buf.len() >= config.max_bytes {
            // Drain without storing
            let mut drain = [0u8; 16 * 1024];
            while reader.read(&mut drain).await? != 0 {
                if cancel.is_cancelled() {
                    return Err(ParseStreamError::Cancelled);
                }
            }
            break;
        }
    }

    // Create document and parse synchronously (the html5 parser is sync)
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&String::from_utf8_lossy(&buf), Some(Encoding::UTF8));
    stream.close();

    let mut doc = <C::DocumentBuilder as DocumentBuilder<C>>::new_document(Some(base_url.clone()));
    let _ = crate::parser::Html5Parser::<C>::parse_document(&mut stream, &mut doc, None);

    // Walk DOM to discover sub-resources
    discover_resources::<C>(&doc, &base_url, &mut on_discover);

    Ok(doc)
}

/// Extract the document title by walking the DOM.
pub fn extract_title<C: HasDocument>(doc: &C::Document) -> Option<String> {
    let root = doc.get_root();
    let children: Vec<NodeId> = root.children().to_vec();
    extract_title_in_children::<C>(doc, &children)
}

fn extract_title_in_children<C: HasDocument>(doc: &C::Document, children: &[NodeId]) -> Option<String> {
    for &node_id in children {
        let Some(node) = doc.node_by_id(node_id) else {
            continue;
        };

        if node.is_element_node() {
            if let Some(el) = node.get_element_data() {
                if el.name().eq_ignore_ascii_case("title") {
                    // Collect text content from children
                    let mut text = String::new();
                    for &text_id in node.children() {
                        let Some(text_node) = doc.node_by_id(text_id) else {
                            continue;
                        };
                        if let NodeData::Text(t) = text_node.data() {
                            text.push_str(t.value());
                        }
                    }
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
            // Recurse
            let grandchildren: Vec<NodeId> = node.children().to_vec();
            if let Some(found) = extract_title_in_children::<C>(doc, &grandchildren) {
                return Some(found);
            }
        }
    }
    None
}

fn discover_resources<C: HasDocument>(doc: &C::Document, base: &Url, on_discover: &mut impl FnMut(ResourceHint)) {
    let root = doc.get_root();
    let children: Vec<NodeId> = root.children().to_vec();
    discover_in_children::<C>(doc, &children, base, on_discover);
}

fn discover_in_children<C: HasDocument>(
    doc: &C::Document,
    children: &[NodeId],
    base: &Url,
    on_discover: &mut impl FnMut(ResourceHint),
) {
    for &node_id in children {
        let Some(node) = doc.node_by_id(node_id) else {
            continue;
        };

        if node.is_element_node() {
            if let Some(el) = node.get_element_data() {
                let tag = el.name().cow_to_lowercase();
                match tag.as_ref() {
                    "link" => {
                        let rel = el.attribute("rel").map(|s| s.cow_to_lowercase());
                        if rel.as_deref() == Some("stylesheet") {
                            if let Some(href) = el.attribute("href") {
                                if let Ok(url) = base.join(href.as_str()) {
                                    on_discover(ResourceHint {
                                        url,
                                        kind: HintKind::Stylesheet,
                                        priority: HintPriority::High,
                                        integrity: el.attribute("integrity").cloned(),
                                        cross_origin: el.attribute("crossorigin").is_some(),
                                    });
                                }
                            }
                        }
                    }
                    "script" => {
                        if let Some(src) = el.attribute("src") {
                            if let Ok(url) = base.join(src.as_str()) {
                                on_discover(ResourceHint {
                                    url,
                                    kind: HintKind::Script {
                                        defer: el.attribute("defer").is_some(),
                                        async_load: el.attribute("async").is_some(),
                                    },
                                    priority: HintPriority::Normal,
                                    integrity: el.attribute("integrity").cloned(),
                                    cross_origin: el.attribute("crossorigin").is_some(),
                                });
                            }
                        }
                    }
                    "img" => {
                        if let Some(src) = el.attribute("src") {
                            if let Ok(url) = base.join(src.as_str()) {
                                on_discover(ResourceHint {
                                    url,
                                    kind: HintKind::Image,
                                    priority: HintPriority::Low,
                                    integrity: None,
                                    cross_origin: el.attribute("crossorigin").is_some(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Recurse into children
            let grandchildren: Vec<NodeId> = node.children().to_vec();
            discover_in_children::<C>(doc, &grandchildren, base, on_discover);
        }
    }
}
