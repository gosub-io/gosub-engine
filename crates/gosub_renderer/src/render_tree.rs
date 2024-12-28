use anyhow::bail;
use gosub_interface::config::{HasHtmlParser, HasRenderTree};
use gosub_interface::css3::CssSystem;
use gosub_interface::document::{Document, DocumentBuilder};
use gosub_interface::document_handle::DocumentHandle;
use gosub_interface::html5::Html5Parser;
use gosub_net::http::fetcher::Fetcher;
use gosub_rendering::render_tree::{generate_render_tree, RenderTree};
use gosub_shared::byte_stream::{ByteStream, Encoding};
use std::fs;
use url::Url;

/// Generates a render tree from the given URL.. if the source is given, the URL is not loaded, but the source HTML is used instead
pub async fn load_html_rendertree<
    C: HasRenderTree<LayoutTree = RenderTree<C>, RenderTree = RenderTree<C>> + HasHtmlParser,
>(
    url: Url,
    source: Option<&str>,
) -> gosub_shared::types::Result<(RenderTree<C>, Fetcher)> {
    let fetcher = Fetcher::new(url.clone());

    let rt = match source {
        Some(source) => load_html_rendertree_source::<C>(url, source)?,
        None => load_html_rendertree_fetcher::<C>(url, &fetcher).await?,
    };

    Ok((rt, fetcher))
}

// Generate a render tree from the given source HTML. THe URL is needed to resolve relative URLs
// and also to set the base URL for the document.
pub fn load_html_rendertree_source<
    C: HasRenderTree<LayoutTree = RenderTree<C>, RenderTree = RenderTree<C>> + HasHtmlParser,
>(
    url: Url,
    source_html: &str,
) -> gosub_shared::types::Result<RenderTree<C>> {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(source_html, Some(Encoding::UTF8));
    stream.close();

    let mut doc_handle = C::DocumentBuilder::new_document(Some(url));
    let parse_errors = C::HtmlParser::parse(&mut stream, DocumentHandle::clone(&doc_handle), None)?;

    for error in parse_errors {
        eprintln!("Parse error: {:?}", error);
    }

    let mut doc = doc_handle.get_mut();

    doc.add_stylesheet(C::CssSystem::load_default_useragent_stylesheet());

    drop(doc);

    generate_render_tree(DocumentHandle::clone(&doc_handle))
}

/// Generates a render tree from the given URL. The complete HTML source is fetched from the URL async.
pub async fn load_html_rendertree_fetcher<
    C: HasRenderTree<LayoutTree = RenderTree<C>, RenderTree = RenderTree<C>> + HasHtmlParser,
>(
    url: Url,
    fetcher: &Fetcher,
) -> gosub_shared::types::Result<RenderTree<C>> {
    let html = if url.scheme() == "http" || url.scheme() == "https" {
        // Fetch the html from the url
        let response = fetcher.get(url.as_ref()).await?;
        if response.status != 200 {
            bail!(format!("Could not get url. Status code {}", response.status));
        }

        String::from_utf8(response.body.clone())?
    } else if url.scheme() == "file" {
        fs::read_to_string(url.as_str().trim_start_matches("file://"))?
    } else {
        bail!("Unsupported url scheme: {}", url.scheme());
    };

    load_html_rendertree_source(url, &html)
}
