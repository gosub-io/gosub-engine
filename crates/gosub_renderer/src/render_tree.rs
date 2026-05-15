use anyhow::bail;
use gosub_interface::config::{HasDocument, HasHtmlParser, HasRenderTree};
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{Document, DocumentType};
use gosub_interface::html5::Html5Parser;
use gosub_net::net::simple::simple_get;
use gosub_rendering::render_tree::generate_render_tree;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use std::fs;
use url::Url;

pub async fn load_html_rendertree<C: HasRenderTree + HasHtmlParser + HasDocument>(
    url: Url,
    source: Option<&str>,
) -> gosub_shared::types::Result<(C::RenderTree, C::Document)> {
    match source {
        Some(source) => load_html_rendertree_source::<C>(url, source),
        None => load_html_rendertree_url::<C>(url).await,
    }
}

pub fn load_html_rendertree_source<C: HasRenderTree + HasHtmlParser + HasDocument>(
    url: Url,
    source_html: &str,
) -> gosub_shared::types::Result<(C::RenderTree, C::Document)> {
    let mut stream = ByteStream::from_str(source_html, Encoding::UTF8);

    let mut doc = C::Document::new(DocumentType::HTML, Some(url));
    let parse_errors = C::HtmlParser::parse(&mut stream, &mut doc, None)?;

    for error in parse_errors {
        eprintln!("Parse error: {error:?}");
    }

    doc.add_stylesheet(C::CssSystem::load_default_useragent_stylesheet());

    Ok((generate_render_tree::<C>(&doc)?, doc))
}

pub async fn load_html_rendertree_url<C: HasRenderTree + HasHtmlParser + HasDocument>(
    url: Url,
) -> gosub_shared::types::Result<(C::RenderTree, C::Document)> {
    let html = if url.scheme() == "http" || url.scheme() == "https" {
        let body = simple_get(&url).await?;
        String::from_utf8(body.to_vec())?
    } else if url.scheme() == "file" {
        fs::read_to_string(url.as_str().trim_start_matches("file://"))?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    load_html_rendertree_source::<C>(url, &html)
}
