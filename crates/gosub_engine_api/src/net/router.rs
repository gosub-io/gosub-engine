use crate::engine::pipeline::css::DummyStylesheet;
use crate::engine::pipeline::font::DummyFont;
use crate::engine::pipeline::js::DummyJsDocument;
use crate::engine::pipeline::Hooks;
use crate::engine::types::PeekBuf;
use crate::engine::UaPolicy;
use crate::html::DummyDocument;
use crate::net::decision::types::BlockReason;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult};
use crate::net::{decide_handling, stream_to_bytes, HandlingDecision, RenderTarget, RequestDestination, SharedBody};
use anyhow::anyhow;
use bytes::Bytes;
use std::sync::Arc;

/// The outcome of routing a fetch result.
#[derive(Debug)]
pub enum RoutedOutcome {
    /// The main document has been parsed and is ready.
    MainDocument(Arc<DummyDocument>),
    /// The resource has been rendered in a viewer (text, image, pdf, etc.).
    ViewerRendered(Bytes),
    /// A download has been started (path to file).
    DownloadStarted(std::path::PathBuf),
    /// A download has finished (path to file).
    DownloadFinished(std::path::PathBuf),

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
    /// The request was cancelled (e.g., navigation away).
    Cancelled,
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
pub async fn route_response_for(
    dest: RequestDestination,
    handle: FetchHandle,
    request: FetchRequest,
    fetch_result: FetchResult,
    policy: &UaPolicy,
    hooks: &mut Hooks,
) -> anyhow::Result<RoutedOutcome> {
    // Fetch the metadata, peek buffer and content (type)
    let (meta, body_content, peek_buf) = match fetch_result {
        FetchResult::Stream { meta, peek_buf, shared } => (meta, BodyContent::Stream { shared }, peek_buf),
        FetchResult::Buffered { meta, body } => {
            let peek_buf = PeekBuf::from_slice(&body[0..5 * 1024]);
            (meta, BodyContent::Buffered { body }, peek_buf)
        }
        FetchResult::Error(e) => {
            return Err(anyhow!(e));
        }
    };

    // Decide what we need to do with the response
    let outcome = decide_handling(&meta, dest, peek_buf.clone(), policy);

    match (dest, outcome.decision, body_content) {
        (RequestDestination::Document, HandlingDecision::Render(target), body_content) => {
            // We need to render it
            match target {
                RenderTarget::TextViewer => Ok(RoutedOutcome::ViewerRendered(
                    body_content.to_bytes(peek_buf.clone()).await?,
                )),
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
                RenderTarget::MediaPipeline => {
                    Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?))
                }
                RenderTarget::FontLoader => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
                RenderTarget::PdfViewer => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
                RenderTarget::BodyToJs => Ok(RoutedOutcome::ViewerRendered(body_content.to_bytes(peek_buf).await?)),
            }
        }
        (RequestDestination::Document, HandlingDecision::Download { .. }, _) => {
            // Download resource if it's a main document
            // let dest = hooks.download.resolve_or_prompt(path, &meta, &outcome).await?;
            // hooks.download.to_file(top, &dest, &meta).await?;
            // You can split Started vs Finished if streaming.
            // RoutedOutcome::DownloadFinished(dest)
            Err(anyhow!("Cannot download main document"))
        }
        (RequestDestination::Document, HandlingDecision::Block(reason), ..) => Ok(RoutedOutcome::Blocked(reason)),
        (RequestDestination::Document, HandlingDecision::Cancel, ..) => Ok(RoutedOutcome::Cancelled),
        (RequestDestination::Document, HandlingDecision::OpenExternal, ..) => {
            // let p = hooks.external.stage_and_open(top, &meta).await?;
            // RoutedOutcome::DownloadStarted(p)
            Err(anyhow!("Cannot open main document in external application"))
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

        // Any other subresource decision that isn’t Render -> block (no download)
        (_, HandlingDecision::Block(reason), _) => Ok(RoutedOutcome::Blocked(reason)),
        (_, HandlingDecision::Cancel, _) => Ok(RoutedOutcome::Cancelled),

        // Safety net: e.g., Download/OpenExternal for sub resources: treat as block
        (_, HandlingDecision::Download { .. } | HandlingDecision::OpenExternal | HandlingDecision::Render(_), _) => {
            Ok(RoutedOutcome::Blocked(BlockReason::Policy))
        }
    }
}
