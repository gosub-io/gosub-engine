use std::io;

use crate::html::{EngineDocument, RenderConfiguration};
use gosub_html5::document::builder::DocumentBuilderImpl;
use gosub_html5::parser::Html5Parser;
use gosub_interface::css3::CssSystem;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::{ByteStream, Encoding};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use url::Url;

/// Errors from buffering and parsing a main document stream.
#[derive(thiserror::Error, Debug)]
pub enum DocumentError {
    /// I/O error while reading the document stream.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// URL parsing error
    #[error("URL error: {0}")]
    Url(#[from] url::ParseError),

    /// Cancellation (navigation cancelled).
    #[error("Cancelled")]
    Cancelled,
}

/// Configuration for parsing a main document (see [`parse_main_document_stream`]).
#[derive(Debug, Clone)]
pub struct HtmlParseConfig {
    /// Max bytes to buffer from the stream; a larger document is truncated (with a warning).
    /// The engine reads this from the `net.document.max_bytes` setting.
    pub max_bytes: usize,
    /// How the parser fetches an external stylesheet. `None` means it does not:
    /// the parser has no network capability of its own (see
    /// `gosub_html5::parser::Html5ParserOptions::resource_loader`).
    pub resource_loader: Option<std::sync::Arc<dyn gosub_interface::resource_loader::ResourceLoader>>,
    /// Also return the document's source text, for an engine that will hand it
    /// to a renderer process (which re-parses; a DOM cannot cross a fork by
    /// value). Off by default - retaining a copy of every document would tax
    /// engines that render in-process.
    pub capture_source: bool,
}

impl Default for HtmlParseConfig {
    fn default() -> Self {
        // Matches the `net.document.max_bytes` schema default.
        Self {
            max_bytes: 10 * 1024 * 1024,
            resource_loader: None,
            capture_source: false,
        }
    }
}

/// Read a document's bytes as source text, without parsing it: what a tab
/// keeps when a renderer process does the parsing. Same size cap and lossy
/// UTF-8 as the captured source of a parsed document.
pub async fn read_document_source<R>(
    base_url: &Url,
    mut reader: R,
    cancel: CancellationToken,
    max_bytes: usize,
) -> Result<std::sync::Arc<str>, DocumentError>
where
    R: AsyncRead + Unpin + Send,
{
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 16 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(DocumentError::Cancelled);
        }
        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        let remaining = max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }
        if buf.len() >= max_bytes {
            log::warn!("Document {base_url} exceeds the {max_bytes} byte limit (net.document.max_bytes); truncated");
            let mut drain = [0u8; 16 * 1024];
            while reader.read(&mut drain).await? != 0 {
                if cancel.is_cancelled() {
                    return Err(DocumentError::Cancelled);
                }
            }
            break;
        }
    }
    Ok(std::sync::Arc::<str>::from(String::from_utf8_lossy(&buf).as_ref()))
}

/// Main entry point: buffer the HTML stream and parse it into a real DOM
/// document. Subresources are fetched by whoever consumes them (the parser's
/// loader for stylesheets, the media store for images) - nothing is
/// prefetched here, since nothing could use a prefetch.
pub async fn parse_main_document_stream<C, R>(
    base_url: Url,
    mut reader: R,
    cancel: CancellationToken,
    cfg: HtmlParseConfig,
) -> Result<(EngineDocument<C>, Option<std::sync::Arc<str>>), DocumentError>
where
    C: RenderConfiguration,
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = Vec::with_capacity(32 * 1024);
    let mut tmp = [0u8; 16 * 1024];

    loop {
        if cancel.is_cancelled() {
            return Err(DocumentError::Cancelled);
        }

        let n = reader.read(&mut tmp).await?;
        if n == 0 {
            break;
        }

        let remaining = cfg.max_bytes.saturating_sub(buf.len()).min(n);
        if remaining > 0 {
            buf.extend_from_slice(&tmp[..remaining]);
        }
        // If we hit the cap, we still drain the stream to EOF quickly
        // to avoid keeping the connection open unnecessarily.
        if buf.len() >= cfg.max_bytes {
            log::warn!(
                "Document {base_url} exceeds the {} byte limit (net.document.max_bytes); parsing truncated content",
                cfg.max_bytes
            );
            let mut drain = [0u8; 16 * 1024];
            while reader.read(&mut drain).await? != 0 {
                if cancel.is_cancelled() {
                    return Err(DocumentError::Cancelled);
                }
            }
            break;
        }
    }

    // The captured source is lossy UTF-8; the parse below decodes properly.
    let source = cfg
        .capture_source
        .then(|| std::sync::Arc::<str>::from(String::from_utf8_lossy(&buf).as_ref()));

    // Detect encoding from the raw bytes (BOM check + chardetng), then build a
    // properly-decoded stream.  We cannot call set_encoding() on an Unknown-
    // encoded stream because tell_bytes() returns buffer.len() when chars is
    // empty, which would advance the position to EOF.
    let encoding = {
        let mut tmp = ByteStream::new(Encoding::Unknown, None);
        tmp.read_from_bytes(&buf)?;
        tmp.detect_encoding()
    };
    let mut stream = ByteStream::new(encoding, None);
    stream.read_from_bytes(&buf)?;
    let mut doc = DocumentBuilderImpl::new_document::<C>(Some(base_url));
    // Hand the parser the loader so `<link rel="stylesheet">` resolves through the
    // broker rather than a socket opened mid-parse.
    let parser_options = gosub_html5::parser::Html5ParserOptions {
        resource_loader: cfg.resource_loader.clone(),
        ..Default::default()
    };
    let _ = Html5Parser::<C>::parse_document(&mut stream, &mut doc, Some(parser_options));
    let ua = <C::CssSystem as CssSystem>::load_default_useragent_stylesheet();
    doc.add_stylesheet(ua);

    Ok((doc, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::DefaultRenderConfig;
    use bytes::Bytes;
    use futures::stream;
    use tokio_util::io::StreamReader;

    fn reader_from_str(s: &str) -> impl AsyncRead + Unpin + Send + 'static {
        // One-chunk stream -> AsyncRead
        let it = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::from(s.to_owned()))]);
        StreamReader::new(it)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parses_the_title() {
        let html = r#"
            <html>
              <head>
                <title> Hello World </title>
                <link rel="stylesheet" href="/style.css">
              </head>
              <body>
                <script src="app.js"></script>
                <img src="images/logo.png">
              </body>
            </html>
        "#;

        let base = Url::parse("https://example.com/path/index.html").unwrap();
        let cancel = CancellationToken::new();

        let (doc, _) = parse_main_document_stream::<DefaultRenderConfig, _>(
            base.clone(),
            reader_from_str(html),
            cancel,
            HtmlParseConfig::default(),
        )
        .await
        .unwrap();
        assert_eq!(crate::html::document_title(&doc).as_deref(), Some("Hello World"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn honors_cancellation() {
        let base = Url::parse("https://e.test/").unwrap();

        // Make a stream that hangs so we can cancel before read completes.
        use futures::stream::pending;
        let pending_stream = pending::<Result<Bytes, io::Error>>();
        let reader = StreamReader::new(pending_stream);

        let cancel = CancellationToken::new();
        cancel.cancel(); // cancel immediately

        let res =
            parse_main_document_stream::<DefaultRenderConfig, _>(base, reader, cancel, HtmlParseConfig::default())
                .await;

        match res {
            Err(DocumentError::Cancelled) => {}
            other => panic!("expected Cancelled, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn truncates_at_max_bytes() {
        let base = Url::parse("https://e.test/").unwrap();
        let big = "A".repeat(150_000); // 150 KiB
        let cfg = HtmlParseConfig {
            max_bytes: 64 * 1024, // 64 KiB
            ..Default::default()
        };

        // Just verify truncated input still produces a valid document (no panic).
        parse_main_document_stream::<DefaultRenderConfig, _>(
            base,
            reader_from_str(&big),
            CancellationToken::new(),
            cfg,
        )
        .await
        .unwrap();
    }
}
