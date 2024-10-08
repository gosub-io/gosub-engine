use gosub_css3::system::Css3System;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::parser::Html5Parser;
use gosub_renderer::render_tree::TreeDrawer;
use gosub_rendering::render_tree::RenderTree;
use gosub_shared::types::Result;
use gosub_taffy::TaffyLayouter;
use gosub_useragent::application::{Application, WindowOptions};
use gosub_vello::VelloBackend;
use js_sys::Promise;
use log::info;
use url::Url;
use wasm_bindgen::prelude::*;

type Backend = VelloBackend;
type Layouter = TaffyLayouter;

type CssSystem = Css3System;

type Document = DocumentImpl<CssSystem>;

type HtmlParser<'a> = Html5Parser<'a, Document, CssSystem>;

type Drawer = TreeDrawer<Backend, Layouter, Document, CssSystem>;
type Tree = RenderTree<Layouter, CssSystem>;

#[wasm_bindgen]
pub struct RendererOptions {
    id: String,
    parent_id: String,
    url: String,
    debug: bool,
}

#[wasm_bindgen]
impl RendererOptions {
    #[wasm_bindgen(constructor)]
    pub fn new(id: String, parent_id: String, url: String, debug: bool) -> Self {
        Self {
            id,
            parent_id,
            url,
            debug,
        }
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
    let mut application: Application<Drawer, Backend, Layouter, Tree, Document, CssSystem, HtmlParser> =
        Application::new(VelloBackend::new().await?, TaffyLayouter, opts.debug);

    info!("created application");

    application.initial_tab(
        Url::parse(&opts.url)?,
        WindowOptions {
            id: opts.id,
            parent_id: opts.parent_id,
        },
    );

    application.initialize()?;

    application.run()?;

    Ok(())
}
