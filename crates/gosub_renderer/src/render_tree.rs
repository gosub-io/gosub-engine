use std::fs;

use anyhow::bail;
use taffy::{Layout, TaffyTree};
use taffy::{NodeId as TaffyID, NodeId};
use url::Url;

use gosub_html5::node::NodeId as GosubID;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_net::http::ureq;
use gosub_render_backend::{RenderBackend, SizeU32};
use gosub_rendering::position::PositionTree;
use gosub_shared::byte_stream::{ByteStream, Confidence, Encoding};
use gosub_styling::css_values::CssProperties;
use gosub_styling::render_tree::{generate_render_tree, RenderNodeData, RenderTree as StyleTree};

pub type NodeID = TaffyID;

pub struct TreeDrawer<B: RenderBackend> {
    pub(crate) style: StyleTree<B>,
    pub(crate) root: NodeID,
    pub(crate) taffy: TaffyTree<GosubID>,
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

impl<B: RenderBackend> TreeDrawer<B> {
    pub fn new(
        style: StyleTree<B>,
        taffy: TaffyTree<GosubID>,
        root: TaffyID,
        url: Url,
        debug: bool,
    ) -> Self {
        let position = PositionTree::from_taffy(&taffy, root);
        Self {
            style,
            root,
            taffy,
            size: None,
            url,
            position,
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
    pub parent: Option<NodeID>,
    pub children: Vec<NodeID>,
    pub layout: Layout,
    pub name: String,
    pub properties: CssProperties,
    pub namespace: Option<String>,
    pub data: RenderNodeData<B>,
}

pub(crate) fn load_html_rendertree<B: RenderBackend>(
    url: Url,
) -> gosub_shared::types::Result<StyleTree<B>> {
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
