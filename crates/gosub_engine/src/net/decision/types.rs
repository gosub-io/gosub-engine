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
    /// Refused by CORS, and by which rule.
    Cors(CorsFailure),
    /// The request required a stored response and the cache had none.
    NotCached,
}

impl BlockReason {
    /// Map the network layer's refusal reason into the engine's vocabulary.
    pub fn from_net(reason: gosub_sonar::net::types::BlockReason) -> Self {
        match reason {
            gosub_sonar::net::types::BlockReason::MixedContent => BlockReason::MixedContent,
            gosub_sonar::net::types::BlockReason::UrlPolicy => BlockReason::UrlPolicy,
            gosub_sonar::net::types::BlockReason::UnsupportedScheme => BlockReason::UnsupportedScheme,
            gosub_sonar::net::types::BlockReason::Cors(error) => BlockReason::Cors(CorsFailure::from_net(error)),
            gosub_sonar::net::types::BlockReason::NotCached => BlockReason::NotCached,
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
            BlockReason::Cors(failure) => return write!(f, "blocked by CORS: {failure}"),
            BlockReason::NotCached => "not in the cache",
        };
        f.write_str(s)
    }
}

/// Which CORS rule refused a request ([Fetch] §3.2, §4.9-4.10).
///
/// The engine's own copy of gosub-sonar's `CorsError`, one variant each, so the rule that
/// failed reaches the embedder rather than a bare "CORS": "blocked by CORS" sends a developer
/// off to read the whole protocol, "no Access-Control-Allow-Origin header" tells them which
/// header to add. Mirrored rather than re-exported, like [`BlockReason`] itself, so sonar can
/// grow a rule without that being a breaking change here.
///
/// Serialisable because a refusal can be classified in the network process and cross to the
/// broker inside its [`LoadError`](crate::engine::LoadError).
///
/// [Fetch]: https://fetch.spec.whatwg.org/#http-cors-protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum CorsFailure {
    /// The response carried no `Access-Control-Allow-Origin` header.
    MissingAllowOrigin,
    /// `Access-Control-Allow-Origin` named another origin, or appeared more than once.
    OriginMismatch,
    /// `Access-Control-Allow-Origin: *` on a request that carried credentials.
    WildcardWithCredentials,
    /// A credentialed request without `Access-Control-Allow-Credentials: true` in the response.
    CredentialsNotAllowed,
    /// A same-origin-mode request was sent, or redirected, to another origin.
    SameOriginMode,
    /// A cross-origin no-cors request used a method other than GET, HEAD or POST.
    UnsafeMethodForNoCors,
    /// A cross-origin no-cors request carried a header that is not CORS-safelisted.
    UnsafeHeaderForNoCors,
    /// The preflight `OPTIONS` came back with a status outside 2xx.
    PreflightStatus,
    /// The preflight's `Access-Control-Allow-Methods` or `-Headers` could not be parsed.
    PreflightInvalidResponse,
    /// The preflight did not allow the request's method.
    PreflightMethodRejected,
    /// The preflight did not allow one of the request's non-safelisted headers.
    PreflightHeaderRejected,
    /// A redirect `Location` carried `user:password` credentials.
    CredentialedRedirect,
}

impl CorsFailure {
    /// Map the network layer's CORS error into the engine's vocabulary.
    pub fn from_net(error: gosub_sonar::net::cors::CorsError) -> Self {
        use gosub_sonar::net::cors::CorsError as Net;
        match error {
            Net::MissingAllowOrigin => CorsFailure::MissingAllowOrigin,
            Net::OriginMismatch => CorsFailure::OriginMismatch,
            Net::WildcardWithCredentials => CorsFailure::WildcardWithCredentials,
            Net::CredentialsNotAllowed => CorsFailure::CredentialsNotAllowed,
            Net::SameOriginMode => CorsFailure::SameOriginMode,
            Net::UnsafeMethodForNoCors => CorsFailure::UnsafeMethodForNoCors,
            Net::UnsafeHeaderForNoCors => CorsFailure::UnsafeHeaderForNoCors,
            Net::PreflightStatus => CorsFailure::PreflightStatus,
            Net::PreflightInvalidResponse => CorsFailure::PreflightInvalidResponse,
            Net::PreflightMethodRejected => CorsFailure::PreflightMethodRejected,
            Net::PreflightHeaderRejected => CorsFailure::PreflightHeaderRejected,
            Net::CredentialedRedirect => CorsFailure::CredentialedRedirect,
        }
    }
}

impl std::fmt::Display for CorsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            CorsFailure::MissingAllowOrigin => "no Access-Control-Allow-Origin header",
            CorsFailure::OriginMismatch => "Access-Control-Allow-Origin does not match the origin",
            CorsFailure::WildcardWithCredentials => {
                "Access-Control-Allow-Origin '*' cannot authorize a credentialed request"
            }
            CorsFailure::CredentialsNotAllowed => "Access-Control-Allow-Credentials is not 'true'",
            CorsFailure::SameOriginMode => "same-origin mode request targeted another origin",
            CorsFailure::UnsafeMethodForNoCors => "method not allowed for a cross-origin no-cors request",
            CorsFailure::UnsafeHeaderForNoCors => "header not allowed for a cross-origin no-cors request",
            CorsFailure::PreflightStatus => "preflight response status was not ok",
            CorsFailure::PreflightInvalidResponse => "preflight response headers could not be parsed",
            CorsFailure::PreflightMethodRejected => "method not allowed by preflight response",
            CorsFailure::PreflightHeaderRejected => "header not allowed by preflight response",
            CorsFailure::CredentialedRedirect => "redirect URL with embedded credentials",
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
