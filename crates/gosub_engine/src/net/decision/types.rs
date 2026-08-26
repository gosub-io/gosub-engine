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
//
// Deliberately minimal: variants for open-externally, block-on-type-mismatch,
// nosniff enforcement and silent cancellation were removed until the features
// that produce them exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlingDecision {
    /// Resource needs to be rendered based on its target (html parser, css parser, js engine, image decoder, etc).
    Render(RenderTarget),
    /// Resource should be downloaded to the given path.
    Download { path: PathBuf },
}

/// Why a load was refused.
///
/// Covers both the engine's own decision path and the refusals the network layer reports;
/// [`from_net`](Self::from_net) maps gosub-sonar's vocabulary into this one, following the
/// same pattern as [`ResourceKind`](crate::net::types::ResourceKind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockReason {
    /// A user agent or site policy explicitly forbids this load.
    /// Example: a CSP violation, or a UA rule against auto-downloads.
    Policy,
    /// An insecure subresource was requested by a secure document.
    MixedContent,
    /// Refused by the URL policy the engine gave the fetcher.
    UrlPolicy,
    /// The URL scheme is not `http` or `https`.
    UnsupportedScheme,
}

impl BlockReason {
    /// Map the network layer's refusal reason into the engine's vocabulary.
    pub fn from_net(reason: gosub_sonar::net::types::BlockReason) -> Self {
        match reason {
            gosub_sonar::net::types::BlockReason::MixedContent => BlockReason::MixedContent,
            gosub_sonar::net::types::BlockReason::UrlPolicy => BlockReason::UrlPolicy,
            gosub_sonar::net::types::BlockReason::UnsupportedScheme => BlockReason::UnsupportedScheme,
        }
    }
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BlockReason::Policy => "blocked by policy",
            BlockReason::MixedContent => "blocked as mixed content",
            BlockReason::UrlPolicy => "blocked by URL policy",
            BlockReason::UnsupportedScheme => "unsupported URL scheme",
        };
        f.write_str(s)
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
    /// Send to the font manager
    FontLoader,
    /// Send to the PDF viewer
    PdfViewer,
}
