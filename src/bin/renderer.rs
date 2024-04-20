use std::fs;

use anyhow::bail;
use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_renderer::render_tree::TreeDrawer;
use gosub_renderer::renderer::{Renderer, RendererOptions};
use gosub_rendering::layout::generate_taffy_tree;
use gosub_shared::bytes::CharIterator;
use gosub_shared::bytes::{Confidence, Encoding};
use gosub_shared::types::Result;
use gosub_styling::render_tree::{generate_render_tree, RenderTree as StyleTree};
use url::Url;

fn main() -> Result<()> {
    let matches = clap::Command::new("Gosub Renderer")
        .arg(
            clap::Arg::new("url")
                .help("The url or file to parse")
                .required(true)
                .index(1),
        )
        .get_matches();

    let url: String = matches.get_one::<String>("url").expect("url").to_string();

    let mut rt = load_html_rendertree(&url)?;

    let (taffy_tree, root) = generate_taffy_tree(&mut rt)?;

    let render_tree = TreeDrawer::new(rt, taffy_tree, root);

    let render_tree = render_tree;

    let renderer = Renderer::new(RendererOptions::default())?;

    renderer.start(render_tree)?;
    Ok(())
}

fn load_html_rendertree(str_url: &str) -> Result<StyleTree> {
    let url = Url::parse(str_url)?;
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
        fs::read_to_string(str_url.trim_start_matches("file://"))?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    let mut chars = CharIterator::new();
    chars.read_from_str(&html, Some(Encoding::UTF8));
    chars.set_confidence(Confidence::Certain);

    let doc_handle = DocumentBuilder::new_document(Some(url));
    let _parse_errors =
        Html5Parser::parse_document(&mut chars, Document::clone(&doc_handle), None)?;

    generate_render_tree(Document::clone(&doc_handle))
}
