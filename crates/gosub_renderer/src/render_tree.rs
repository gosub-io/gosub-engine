use std::fs;

use anyhow::bail;
use url::Url;

use gosub_html5::node::NodeId;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_net::http::ureq;
use gosub_render_backend::geo::SizeU32;
use gosub_render_backend::layout::Layouter;
use gosub_render_backend::RenderBackend;
use gosub_rendering::position::PositionTree;
use gosub_shared::byte_stream::{ByteStream, Confidence, Encoding};
use gosub_styling::css_values::CssProperties;
use gosub_styling::render_tree::{generate_render_tree, RenderNodeData, RenderTree};

pub struct TreeDrawer<B: RenderBackend, L: Layouter> {
    pub(crate) tree: RenderTree<B, L>,
    pub(crate) layouter: L,
    pub(crate) size: Option<SizeU32>,
    pub(crate) url: Url,
    pub(crate) position: PositionTree,
    pub(crate) last_hover: Option<NodeId>,
    pub(crate) debug: bool,
    pub(crate) dirty: bool,
    pub(crate) debugger_scene: Option<B::Scene>,
    pub(crate) tree_scene: Option<B::Scene>,
    pub(crate) scene_transform: Option<B::Transform>,
}

impl<B: RenderBackend, L: Layouter> TreeDrawer<B, L> {
    pub fn new(tree: RenderTree<B, L>, layouter: L, url: Url, debug: bool) -> Self {
        Self {
            tree,
            layouter,
            size: None,
            url,
            position: PositionTree::default(),
            last_hover: None,
            debug,
            debugger_scene: None,
            dirty: false,
            tree_scene: None,
            scene_transform: None,
        }
    }
}

pub struct RenderTreeNode<B: RenderBackend> {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub layout: i32, //TODO
    pub name: String,
    pub properties: CssProperties,
    pub namespace: Option<String>,
    pub data: RenderNodeData<B>,
}

pub(crate) fn load_html_rendertree<B: RenderBackend, L: Layouter>(
    url: Url,
) -> gosub_shared::types::Result<RenderTree<B, L>> {
    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let response = ureq::get(url.as_ref()).call()?;
        if response.status() != 200 {
            bail!(format!(
                "Could not get url. Status code {}",
                response.status()
            ));
        }
        response.into_string()?
    } else if url.scheme() == "file" {
        fs::read_to_string(url.as_str().trim_start_matches("file://"))?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    let mut stream = ByteStream::new();
    stream.read_from_str(&html, Some(Encoding::UTF8));
    stream.set_confidence(Confidence::Certain);
    stream.close();

    let mut doc_handle = DocumentBuilder::new_document(Some(url));
    let parse_errors =
        Html5Parser::parse_document(&mut stream, Document::clone(&doc_handle), None)?;

    for error in parse_errors {
        eprintln!("Parse error: {:?}", error);
    }

    let mut doc = doc_handle.get_mut();
    doc.stylesheets
        .push(gosub_styling::load_default_useragent_stylesheet()?);

    drop(doc);

    generate_render_tree(Document::clone(&doc_handle))
}
