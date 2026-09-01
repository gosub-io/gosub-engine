//! Serving `file://` URLs from the local filesystem.
//!
//! gosub-sonar only speaks http(s) (anything else is `BlockReason::UnsupportedScheme`), so
//! the engine serves the file scheme itself: the I/O thread intercepts file requests before
//! they reach the zone fetcher and answers them from disk with a synthesized buffered
//! response. The rest of the pipeline (content routing, parsing, subresource discovery,
//! download offers) treats the result like any other fetch.
//!
//! Access policy — enforced per request in [`refusal`]:
//! - `net.file.enabled` (settings store) turns the scheme off entirely.
//! - Top-level navigations (`ResourceKind::Primary`) are always served. Note this is laxer
//!   than mainstream browsers, which also refuse file links *clicked from* remote pages;
//!   the request does not carry enough context to tell a typed URL from a clicked link.
//! - User downloads (`RequestReference::Download`) are served.
//! - Subresources are served only when the initiating document is itself `file://` (the
//!   engine stamps `FetchRequest::referrer` with the document URL; gosub-sonar never turns
//!   a non-http(s) referrer into a `Referer` header, so this cannot leak local paths).
//! - Everything else is refused: a remote page can never read local files.
//!
//! Directories render as a generated listing page; unknown file types are served as
//! `application/octet-stream`, which flows into the regular download offer.

use crate::net::req_ref_tracker::{RequestReference, REF_REGISTRY};
use crate::net::types::{FetchRequest, FetchResult, FetchResultMeta, NetError};
use bytes::Bytes;
use cow_utils::CowUtils;
use gosub_sonar::net::events::NetEvent;
use gosub_sonar::net::observer::NetObserver;
use gosub_sonar::net::types::{BlockReason, ResourceKind as NetResourceKind};
use http::HeaderMap;
use std::path::Path;
use std::sync::Arc;
use url::Url;

/// Whether this request is for the engine-served `file://` scheme.
pub fn handles(req: &FetchRequest) -> bool {
    req.url.scheme() == "file"
}

/// Why a file request is refused, if it is.
fn refusal(req: &FetchRequest, enabled: bool) -> Option<BlockReason> {
    if !enabled {
        return Some(BlockReason::UrlPolicy);
    }
    // Top-level navigation (address bar, link click).
    if req.kind == NetResourceKind::Primary {
        return None;
    }
    // A user-initiated download ("save link as" on a file link).
    if matches!(
        REF_REGISTRY.from_net(req.reference),
        Some(RequestReference::Download(_))
    ) {
        return None;
    }
    // Subresource of a document that is itself served from disk.
    if req.referrer.as_ref().is_some_and(|r| r.scheme() == "file") {
        return None;
    }
    Some(BlockReason::UrlPolicy)
}

/// Serve a `file://` request, emitting the same observer events a network fetch would, and
/// returning the buffered response (or error) for the reply channel.
pub async fn serve(req: &FetchRequest, enabled: bool, observer: Arc<dyn NetObserver + Send + Sync>) -> FetchResult {
    let url = req.url.clone();

    if let Some(reason) = refusal(req, enabled) {
        log::warn!(
            "refusing file:// request for {url} (kind {:?}, referrer {:?})",
            req.kind,
            req.referrer
        );
        observer.on_event(NetEvent::Blocked {
            url: url.clone(),
            reason,
        });
        return FetchResult::Error(NetError::Blocked { reason, url });
    }

    observer.on_event(NetEvent::Started { url: url.clone() });
    let started = std::time::Instant::now();

    match load(&url).await {
        Ok((content_type, body)) => {
            let mut headers = HeaderMap::new();
            if let Ok(v) = content_type.parse() {
                headers.insert(http::header::CONTENT_TYPE, v);
            }
            if let Ok(v) = body.len().to_string().parse() {
                headers.insert(http::header::CONTENT_LENGTH, v);
            }
            observer.on_event(NetEvent::ResponseHeaders {
                url: url.clone(),
                status: 200,
                headers: headers.clone(),
            });
            let len = body.len() as u64;
            observer.on_event(NetEvent::Progress {
                received_bytes: len,
                expected_length: Some(len),
                elapsed: started.elapsed(),
            });
            observer.on_event(NetEvent::Finished {
                received_bytes: len,
                elapsed: started.elapsed(),
                url: url.clone(),
            });
            FetchResult::Buffered {
                meta: {
                    let mut meta = FetchResultMeta::synthetic(url);
                    meta.headers = headers;
                    meta.content_length = Some(len);
                    meta.content_type = Some(content_type.to_string());
                    meta.has_body = len > 0;
                    meta
                },
                body,
            }
        }
        Err(e) => {
            let err = NetError::Io(Arc::new(e));
            observer.on_event(NetEvent::Failed {
                url,
                error: anyhow::anyhow!(err.clone()),
            });
            FetchResult::Error(err)
        }
    }
}

/// Read the URL's path from disk: file contents, or a generated listing for a directory.
async fn load(url: &Url) -> std::io::Result<(&'static str, Bytes)> {
    let path = url
        .to_file_path()
        .map_err(|()| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a local file path"))?;
    let meta = tokio::fs::metadata(&path).await?;
    if meta.is_dir() {
        let listing = directory_listing(&path).await?;
        Ok(("text/html; charset=utf-8", Bytes::from(listing)))
    } else {
        let body = tokio::fs::read(&path).await?;
        Ok((content_type_for(&path), Bytes::from(body)))
    }
}

/// `Content-Type` by file extension. `application/octet-stream` for anything unknown,
/// which routes into the download offer like a server would trigger it.
fn content_type_for(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ext = ext.cow_to_ascii_lowercase();
    match ext.as_ref() {
        "html" | "htm" | "xhtml" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" | "text" | "log" | "md" | "rs" | "toml" | "yaml" | "yml" | "ini" | "csv" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "wasm" => "application/wasm",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        _ => "application/octet-stream",
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn human_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KIB * KIB * KIB {
        format!("{:.1} GiB", b / (KIB * KIB * KIB))
    } else if b >= KIB * KIB {
        format!("{:.1} MiB", b / (KIB * KIB))
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// A generated index page for a directory, in the internal-pages house style.
/// Tabular data is inline-block rows, not `<table>` (see the internal_pages module:
/// the current auto table layout misplaces columns until the lattice work merges).
async fn directory_listing(path: &Path) -> std::io::Result<String> {
    struct Entry {
        name: String,
        href: Option<Url>,
        is_dir: bool,
        size: u64,
    }

    let mut entries = Vec::new();
    let mut rd = tokio::fs::read_dir(path).await?;
    while let Some(item) = rd.next_entry().await? {
        let name = item.file_name().to_string_lossy().into_owned();
        let meta = item.metadata().await;
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.map(|m| m.len()).unwrap_or(0);
        let href = if is_dir {
            Url::from_directory_path(item.path()).ok()
        } else {
            Url::from_file_path(item.path()).ok()
        };
        entries.push(Entry {
            name,
            href,
            is_dir,
            size,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.cow_to_lowercase().cmp(&b.name.cow_to_lowercase()))
    });

    let shown = escape(&path.to_string_lossy());
    let mut rows = String::new();
    if let Some(parent) = path.parent() {
        if let Ok(up) = Url::from_directory_path(parent) {
            rows.push_str(&format!(
                "<div class=\"gr\"><span style=\"width:420px\"><a href=\"{up}\">..</a></span><span></span></div>"
            ));
        }
    }
    for e in entries {
        let label = if e.is_dir {
            format!("{}/", e.name)
        } else {
            e.name.clone()
        };
        let cell = match &e.href {
            Some(href) => format!("<a href=\"{href}\">{}</a>", escape(&label)),
            None => escape(&label),
        };
        let size = if e.is_dir { String::new() } else { human_size(e.size) };
        rows.push_str(&format!(
            "<div class=\"gr\"><span style=\"width:420px\">{cell}</span><span class=\"muted\">{size}</span></div>"
        ));
    }

    Ok(format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Index of {shown}</title><style>\
         body{{margin:0;padding:32px 40px;font-family:sans-serif;font-size:14px;color:#1c2333;background:#ffffff}}\
         h1{{font-size:20px;margin:0 0 16px 0}}\
         .gr span{{display:inline-block;padding:3px 14px 3px 0;vertical-align:top}}\
         a{{color:#1d5fd1;text-decoration:none}} .muted{{color:#8a94a6}}\
         </style></head><body><h1>Index of {shown}</h1>{rows}</body></html>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gosub_sonar::net::null_emitter::NullEmitter;
    use gosub_sonar::net::types::FetchRequestBuilder;
    use gosub_sonar::types::RequestId;
    use http::Method;

    fn observer() -> Arc<dyn NetObserver + Send + Sync> {
        Arc::new(NullEmitter)
    }

    fn request(url: &Url) -> FetchRequestBuilder {
        FetchRequest::builder(Method::GET, url.clone()).with_req_id(RequestId::new())
    }

    fn nav_request(url: &Url) -> FetchRequest {
        request(url).with_kind(NetResourceKind::Primary).build()
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gosub-file-loader-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn navigation_serves_a_local_file_with_content_type() {
        let dir = temp_dir();
        let file = dir.join("page.html");
        std::fs::write(&file, "<html><body>hello</body></html>").unwrap();

        let url = Url::from_file_path(&file).unwrap();
        let result = serve(&nav_request(&url), true, observer()).await;
        match result {
            FetchResult::Buffered { meta, body } => {
                assert_eq!(meta.status, 200);
                assert_eq!(meta.content_type.as_deref(), Some("text/html; charset=utf-8"));
                assert!(std::str::from_utf8(&body).unwrap().contains("hello"));
            }
            other => panic!("expected buffered response, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn directory_navigation_renders_a_listing() {
        let dir = temp_dir();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let url = Url::from_directory_path(&dir).unwrap();
        let result = serve(&nav_request(&url), true, observer()).await;
        match result {
            FetchResult::Buffered { meta, body } => {
                let html = std::str::from_utf8(&body).unwrap();
                assert_eq!(meta.content_type.as_deref(), Some("text/html; charset=utf-8"));
                assert!(html.contains("a.txt"));
                assert!(html.contains("sub/"));
                assert!(html.contains("Index of"));
            }
            other => panic!("expected listing, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn missing_file_is_an_io_error() {
        let url = Url::parse("file:///definitely/not/a/real/path/x.html").unwrap();
        let result = serve(&nav_request(&url), true, observer()).await;
        assert!(matches!(result, FetchResult::Error(NetError::Io(_))));
    }

    #[tokio::test]
    async fn disabled_setting_blocks_even_navigations() {
        let url = Url::parse("file:///etc/hostname").unwrap();
        let result = serve(&nav_request(&url), false, observer()).await;
        assert!(matches!(
            result,
            FetchResult::Error(NetError::Blocked {
                reason: BlockReason::UrlPolicy,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn remote_page_cannot_pull_local_subresources() {
        let dir = temp_dir();
        let file = dir.join("secret.txt");
        std::fs::write(&file, "s3cr3t").unwrap();
        let url = Url::from_file_path(&file).unwrap();

        // Subresource (Asset) with a remote referrer: refused.
        let req = request(&url)
            .with_kind(NetResourceKind::Asset)
            .with_referrer(Url::parse("https://evil.example/").unwrap())
            .build();
        assert!(matches!(
            serve(&req, true, observer()).await,
            FetchResult::Error(NetError::Blocked { .. })
        ));

        // Subresource with no referrer at all: refused too.
        let req = request(&url).with_kind(NetResourceKind::Asset).build();
        assert!(matches!(
            serve(&req, true, observer()).await,
            FetchResult::Error(NetError::Blocked { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn file_page_subresources_are_served() {
        let dir = temp_dir();
        let css = dir.join("style.css");
        std::fs::write(&css, "body{color:red}").unwrap();
        let url = Url::from_file_path(&css).unwrap();

        let doc = Url::from_file_path(dir.join("index.html")).unwrap();
        let req = request(&url)
            .with_kind(NetResourceKind::Asset)
            .with_referrer(doc)
            .build();
        match serve(&req, true, observer()).await {
            FetchResult::Buffered { meta, .. } => {
                assert_eq!(meta.content_type.as_deref(), Some("text/css; charset=utf-8"));
            }
            other => panic!("expected buffered response, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn content_types_cover_the_common_cases() {
        assert_eq!(content_type_for(Path::new("x.html")), "text/html; charset=utf-8");
        assert_eq!(content_type_for(Path::new("x.PNG")), "image/png");
        assert_eq!(content_type_for(Path::new("x.unknown")), "application/octet-stream");
        assert_eq!(content_type_for(Path::new("no-extension")), "application/octet-stream");
    }
}
