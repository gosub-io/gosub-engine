use crate::net::decision::sniff::ResponseClass;
use mime::Mime;
use std::path::PathBuf;

/// The context in which the request was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestDestination {
    Document,
    Image,
    Style,
    Script,
    Font,
    Audio,
    Video,
    Worker,
    SharedWorker,
    ServiceWorker,
    Manifest,
    Track,
    Xslt,
    Fetch,
    Xhr,
    Other,
}

/// The outcome of the decision process for handling a response.
#[derive(Debug, Clone)]
pub struct DecisionOutcome {
    /// The coarse class of the response, based on sniffing and/or declared MIME type.
    pub class: ResponseClass,
    /// The coarse class of the response, based on sniffing only (if sniffing was performed).
    pub sniffed_class: Option<ResponseClass>,
    /// The declared MIME type from the `Content-Type` header, if any and parseable.
    pub declared_mime: Option<Mime>,
    /// Whether the response had a `Content-Disposition: attachment` header.
    pub disposition_attachment: bool,
    /// The final decision on how to handle the response.
    pub decision: HandlingDecision,
}

// Final decision for the response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlingDecision {
    /// Resource needs to be rendered based on its target (html parser, css parser, js engine, image decoder, etc).
    Render(RenderTarget),
    /// Resource should be downloaded to the given path.
    Download { path: PathBuf },
    /// Resource should be opened externally (e.g. PDF in external viewer).
    OpenExternal,
    /// Resource should be blocked for the given reason.
    Block(BlockReason),
    /// Resource should be cancelled (aborted silently).
    Cancel,
}

/// Reason on why the response was blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// The resource’s MIME type (declared/sniffed) is incompatible with the request destination.
    /// Example: `<img>` got back `text/html`.
    TypeMismatch,
    /// The response had `X-Content-Type-Options: nosniff`, and the declared MIME type
    /// was missing or not one of the allowed safe types for this destination.
    /// Example: `<script>` got back `text/plain; nosniff`.
    NosniffMismatch,
    /// The response MIME type was present but not recognized or supported by the engine.
    /// Example: `application/vnd.ms-excel` with no registered handler.
    TypeUnknown,
    /// A user agent or site policy explicitly forbids this load.
    /// Example: mixed-content block, CSP violation, or UA rule against auto-downloads.
    Policy,
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockReason::TypeMismatch => write!(f, "type mismatch"),
            BlockReason::NosniffMismatch => write!(f, "nosniff mismatch"),
            BlockReason::TypeUnknown => write!(f, "unknown type"),
            BlockReason::Policy => write!(f, "policy block"),
        }
    }
}

// Where to send the stream if we let the engine render it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderTarget {
    /// Send to the HTML parser (or XHTML parser).
    HtmlParser,
    /// Send to the CSS parser.
    CssParser,
    /// Send to the JavaScript engine.
    JsEngine,
    /// Send to the image decoder.
    ImageDecoder,
    /// Send to the audio pipeline.
    MediaPipeline,
    /// Send to the font manager
    FontLoader,
    /// Send to the PDF viewer
    PdfViewer,
    /// Send to a generic text viewer (for `text/plain` and similar)
    TextViewer,
    // fetch/xhr -> JS
    BodyToJs,
}
