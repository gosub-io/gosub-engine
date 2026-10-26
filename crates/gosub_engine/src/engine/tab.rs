mod handle;
pub mod history;
mod options;
pub(crate) mod remote_effects;
mod scroll;
pub mod services;
mod sink;
mod state;
#[allow(clippy::module_inception)]
mod tab;
mod worker;

pub use handle::TabHandle;
pub use tab::*;

pub use options::TabCookieJar;
pub(crate) use options::TabDefaults;
pub(crate) use options::TabOverrides;
pub use options::TabStorageScope;

pub use sink::TabSink;

pub use history::{HistoryEntryId, HistoryEntrySummary, HistorySnapshot};

/// Whether a page at `from` may take its tab to `to` - by a link, a form, or a
/// navigation its renderer asks for: to the web, from a file page to another file,
/// from an internal page to another internal page, or from a page served over one of
/// the engine's own protocols (`net::protocols`) to another on the same one. Never from
/// anything else to an internal page, `data:`, `javascript:` or anything else; those are
/// reached from the address bar or not at all.
pub(crate) fn page_may_navigate(from: &url::Url, to: &url::Url) -> bool {
    let internal = |url: &url::Url| matches!(url.scheme(), "gosub" | "about");
    matches!(to.scheme(), "http" | "https")
        || (to.scheme() == "file" && from.scheme() == "file")
        || (internal(to) && internal(from))
        || (to.scheme() == from.scheme() && crate::net::protocols::lookup(to.scheme()).is_some())
}

// Tab management and tab-related types.

#[cfg(test)]
mod tests {
    use super::page_may_navigate;
    use url::Url;

    fn may(from: &str, to: &str) -> bool {
        page_may_navigate(&Url::parse(from).unwrap(), &Url::parse(to).unwrap())
    }

    #[test]
    fn a_web_page_reaches_the_web_only() {
        assert!(may("https://a.test/", "https://b.test/x"));
        assert!(may("https://a.test/", "http://b.test/x"));
        for to in [
            "file:///",
            "data:text/html,<h1>x",
            "gosub://history",
            "about:blank",
            "javascript:alert(1)",
        ] {
            assert!(!may("https://a.test/", to), "{to}");
        }
    }

    #[test]
    fn a_file_page_also_reaches_files() {
        assert!(may("file:///home/u/a.html", "file:///home/u/b.html"));
        assert!(may("file:///home/u/a.html", "https://b.test/"));
        assert!(!may("file:///home/u/a.html", "gosub://settings"));
    }

    #[test]
    fn an_internal_page_also_reaches_internal_pages() {
        assert!(may("gosub://home", "gosub://help"));
        assert!(may("gosub://nope", "gosub://history"));
        assert!(may("about:blank", "gosub://config"));
        assert!(may("gosub://home", "https://b.test/"));
        assert!(!may("gosub://home", "file:///etc/passwd"));
        assert!(!may("gosub://home", "javascript:alert(1)"));
    }

    #[cfg(feature = "gopher")]
    #[test]
    fn a_gopher_page_also_reaches_gopherspace() {
        assert!(may("gopher://a.test/1/", "gopher://b.test/7/find?q=x"));
        assert!(may("gopher://a.test/1/", "https://b.test/"));
        assert!(!may("gopher://a.test/1/", "file:///etc/passwd"));
        assert!(!may("gopher://a.test/1/", "gosub://settings"));
        for from in ["https://a.test/", "file:///home/u/a.html", "gosub://home"] {
            assert!(!may(from, "gopher://b.test/1/"), "{from}");
        }
    }

    /// A finger page links nowhere, so no page reaches finger: not the web, and not another
    /// engine protocol's page either (a gopher `h` item is any HTML).
    #[cfg(feature = "finger")]
    #[test]
    fn no_page_reaches_finger() {
        for from in [
            "https://a.test/",
            "file:///home/u/a.html",
            "gosub://home",
            "gopher://a.test/h/page.html",
        ] {
            assert!(!may(from, "finger://b.test/bob"), "{from}");
        }
    }
}
