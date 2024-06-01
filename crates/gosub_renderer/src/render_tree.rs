use anyhow::bail;
use std::fs;
use taffy::{Layout, TaffyTree};
use taffy::{NodeId as TaffyID, NodeId};
use url::Url;

use gosub_html5::node::NodeId as GosubID;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_net::http::ureq;
use gosub_render_backend::{RenderBackend, SizeU32};
use gosub_rendering::position::PositionTree;
use gosub_shared::bytes::{CharIterator, Confidence, Encoding};
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
}

impl<B: RenderBackend> TreeDrawer<B> {
    pub fn new(style: StyleTree<B>, taffy: TaffyTree<GosubID>, root: TaffyID, url: Url) -> Self {
        let position = PositionTree::from_taffy(&taffy, root);
        Self {
            style,
            root,
            taffy,
            size: None,
            url,
            position,
            last_hover: None,
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
        fs::read_to_string(&url.path()[1..])?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    let mut chars = CharIterator::new();
    chars.read_from_str(&html, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);

    let mut doc_handle = DocumentBuilder::new_document(Some(url));
    let _parse_errors =
        Html5Parser::parse_document(&mut chars, Document::clone(&doc_handle), None)?;

    let mut doc = doc_handle.get_mut();
    doc.stylesheets
        .push(gosub_styling::load_default_useragent_stylesheet()?);

    println!("stylesheets: {:?}", doc.stylesheets.len());

    for stylesheet in doc.stylesheets.iter() {
        println!("stylesheet: {:?}", stylesheet.location);
    }

    drop(doc);

    generate_render_tree(Document::clone(&doc_handle))
}
