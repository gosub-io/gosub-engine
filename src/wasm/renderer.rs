use js_sys::Promise;
use url::Url;
use wasm_bindgen::prelude::*;

use gosub_html5::parser::document::{Document, DocumentBuilder};
use gosub_html5::parser::Html5Parser;
use gosub_renderer::render_tree::TreeDrawer;
use gosub_renderer::renderer::{Renderer, RendererOptions as GRendererOptions};
use gosub_shared::types::Result;
use gosub_styling::render_tree::generate_render_tree;
use gosub_styling::render_tree::RenderTree as StyleTree;
use gosub_taffy::layout::generate_taffy_tree;

#[wasm_bindgen]
pub struct RendererOptions {
    id: String,
    html: String,
    url: String,
}

#[wasm_bindgen]
impl RendererOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(id: String, html: String, url: String) -> Self {
        Self { id, html, url }
    }
}

#[wasm_bindgen]
pub struct RendererOutput {
    successful: bool,
    errors: String,
    promise: Promise,
}

#[wasm_bindgen]
impl RendererOutput {
    pub fn is_successful(&self) -> bool {
        self.successful
    }

    pub fn get_errors(&self) -> String {
        self.errors.clone()
    }

    pub fn get_promise(&self) -> Promise {
        self.promise.clone()
    }
}

impl RendererOutput {
    pub fn ok(promise: Promise) -> Self {
        Self {
            successful: true,
            errors: String::new(),
            promise,
        }
    }
}

#[wasm_bindgen]
pub fn renderer(opts: RendererOptions) -> RendererOutput {
    let promise = wasm_bindgen_futures::future_to_promise(async {
        if let Err(e) = renderer_internal(opts).await {
            return Err(JsValue::from_str(&format!("{}", e)));
        };
        Ok(JsValue::NULL)
    });

    RendererOutput::ok(promise)
}

async fn renderer_internal(opts: RendererOptions) -> Result<()> {
    let url = Url::parse(&opts.url)?;

    let mut rt = load_html_rendertree(&opts.html, url)?;

    let (taffy_tree, root) = generate_taffy_tree(&mut rt)?;

    let render_tree = TreeDrawer::new(rt, taffy_tree, root, opts.debug);

    let render_tree = render_tree;

    let renderer = Renderer::new(GRendererOptions::default()).await?;

    renderer.start(render_tree, Some(opts.id))
}

fn load_html_rendertree(input: &str, url: Url) -> Result<StyleTree> {
    let mut stream = ByteStream::new(Encoding::UTF8, None);
    stream.read_from_str(&input, Some(Encoding::UTF8));
    stream.close();

    let doc_handle = DocumentBuilder::new_document(Some(url));
    let _parse_errors =
        Html5Parser::parse_document(&mut stream, Document::clone(&doc_handle), None)?;

    generate_render_tree(Document::clone(&doc_handle))
}
