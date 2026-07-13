use crate::engine::resource_pipeline::css::DummyStylesheet;
use crate::engine::resource_pipeline::font::DummyFont;
use crate::engine::resource_pipeline::js::DummyJsDocument;
use crate::engine::resource_pipeline::ResourcePipelines;
use crate::engine::types::PeekBuf;
use crate::engine::UaPolicy;
use crate::html::{EngineDocument, RenderConfiguration};
use crate::net::decision::types::BlockReason;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::net::{decide_handling, stream_to_bytes, HandlingDecision, RenderTarget, RequestDestination, SharedBody};
use anyhow::anyhow;
use bytes::Bytes;
use std::sync::Arc;

/// The outcome of routing a fetch result.
#[derive(Debug)]
pub enum RoutedOutcome<C: RenderConfiguration> {
    /// The main document has been parsed and is ready.
    MainDocument(Arc<EngineDocument<C>>),
    /// The resource has been rendered in a viewer (text, image, pdf, etc.).
    ViewerRendered(Bytes),

    /// A stylesheet has been loaded and parsed.
    CssLoaded(DummyStylesheet),
    /// A script has been loaded and executed.
    ScriptExecuted(DummyJsDocument),
    /// An image has been decoded.
    ImageDecoded(image::DynamicImage),
    /// A font has been loaded.
    FontLoaded(DummyFont),

    /// The request was blocked (with reason).
    Blocked(BlockReason),
}

/// BodyContent represents either a streaming body or a fully buffered body.
enum BodyContent {
    Stream { shared: Arc<SharedBody> },
    Buffered { body: Bytes },
}

impl BodyContent {
    // Convert to bytes, collecting the stream if necessary. Will take the peek buffer into account (if needed)
    #[allow(clippy::wrong_self_convention)]
    async fn to_bytes(self, peek_buf: PeekBuf) -> anyhow::Result<Bytes> {
        match self {
            BodyContent::Stream { shared } => {
                let buf = stream_to_bytes(peek_buf.clone(), shared).await?;
                Ok(buf)
            }
            BodyContent::Buffered { body } => Ok(body),
        }
    }
}

/// Route a fetch result based on its destination and the UA policy.
pub async fn route_response_for<C: RenderConfiguration>(
    dest: RequestDestination,
    handle: FetchHandle,
    request: FetchRequest,
    fetch_result: FetchResult,
    policy: &UaPolicy,
    hooks: &mut ResourcePipelines<C>,
) -> anyhow::Result<RoutedOutcome<C>> {
    // Fetch the metadata, peek buffer and content (type)
    let (meta, body_content, peek_buf) = match fetch_result {
        FetchResult::Stream { meta, peek_buf, shared } => (meta, BodyContent::Stream { shared }, peek_buf),
        FetchResult::Buffered { meta, body } => {
            let peek_len = body.len().min(5 * 1024);
            let peek_buf = PeekBuf::from_slice(&body[0..peek_len]);
            (meta, BodyContent::Buffered { body }, peek_buf)
        }
        FetchResult::Error(e) => {
            return Err(anyhow!(e));
        }
    };

    // Decide what we need to do with the response
    let outcome = decide_handling(&meta, dest, peek_buf.clone(), policy);

    match (dest, outcome.decision, body_content) {
        (RequestDestination::Document, HandlingDecision::Render(target), body_content) => match target {
            RenderTarget::HtmlParser => {
                let doc = match body_content {
                    BodyContent::Stream { shared } => {
                        hooks.html.parse_stream(request, handle, meta, peek_buf, shared).await?
                    }
                    BodyContent::Buffered { body } => {
                        hooks.html.parse_bytes(request, handle, meta, body.as_ref()).await?
                    }
                };
                Ok(RoutedOutcome::MainDocument(Arc::new(doc)))
            }
            RenderTarget::CssParser => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::JsEngine => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::ImageDecoder => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::FontLoader => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            RenderTarget::PdfViewer => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
        },
        (RequestDestination::Document, HandlingDecision::Download { .. }, _) => {
            Err(anyhow!("Cannot download main document"))
        }

        // -------- Sub resources (no UA prompts) --------
        (RequestDestination::Style, HandlingDecision::Render(RenderTarget::CssParser), body_content) => {
            let stylesheet = match body_content {
                BodyContent::Stream { shared } => hooks.css.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.css.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::CssLoaded(stylesheet))
        }
        (RequestDestination::Script, HandlingDecision::Render(RenderTarget::JsEngine), body_content) => {
            let script = match body_content {
                BodyContent::Stream { shared } => hooks.js.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.js.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::ScriptExecuted(script))
        }
        (RequestDestination::Image, HandlingDecision::Render(RenderTarget::ImageDecoder), body_content) => {
            let image = match body_content {
                BodyContent::Stream { shared } => hooks.images.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.images.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::ImageDecoded(image))
        }
        (RequestDestination::Font, HandlingDecision::Render(RenderTarget::FontLoader), body_content) => {
            let font = match body_content {
                BodyContent::Stream { shared } => hooks.fonts.parse_stream(meta, peek_buf, shared).await?,
                BodyContent::Buffered { body } => hooks.fonts.parse_bytes(meta, body.as_ref()).await?,
            };
            Ok(RoutedOutcome::FontLoaded(font))
        }

        // Safety net: any other subresource decision (Download, or a Render target that
        // doesn't match the destination) is treated as a policy block.
        (_, HandlingDecision::Download { .. } | HandlingDecision::Render(_), _) => {
            Ok(RoutedOutcome::Blocked(BlockReason::Policy))
        }
    }
}
