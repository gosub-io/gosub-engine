//! `gosub://` internal pages - the engine's own scheme handler.
//!
//! `gosub://<name>` (and the `about:<name>` alias) never touches the network: the tab worker
//! resolves it through the [`InternalPages`] registry, which returns HTML that then flows
//! through the ordinary pipeline (parse, layout, history, title). Every embedder therefore
//! gets the same internal pages, and links to them work like any other link.
//!
//! The registry is seeded with the engine's built-in pages and is open to embedders:
//! [`InternalPages::register`] adds a page or **overrides** a built-in one (a branded
//! `home`, say). Providers are plain closures fed a [`PageRequest`], so a page can be static
//! HTML or rendered from live state - the request carries the settings store and a
//! per-tab [`TabView`] snapshot for that.
//!
//! Pages are read-only until the engine can do forms; `gosub://config` is a dump for now.

use crate::tab::HistorySnapshot;
use gosub_config::Config;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

/// What a page provider gets to render from.
pub struct PageRequest<'a> {
    /// Page name: `gosub://version` → `"version"`; also `about:version` → `"version"`.
    pub name: &'a str,
    /// The full URL being navigated to (query string etc. available to providers).
    pub url: &'a Url,
    /// The engine's settings store.
    pub settings: &'a Config,
    /// Snapshot of the requesting tab.
    pub tab: &'a TabView,
}

/// Per-tab state exposed to page providers, captured by the worker at request time.
#[derive(Debug, Clone, Default)]
pub struct TabView {
    /// The tab's session history.
    pub history: HistorySnapshot,
    /// Name of the active render backend (`RenderBackend::name`).
    pub render_backend: &'static str,
}

/// A rendered internal page. Only HTML for now; a `content_type` field is the obvious
/// extension when a page wants to serve e.g. JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageResponse {
    pub html: String,
}

impl PageResponse {
    pub fn html(html: impl Into<String>) -> Self {
        Self { html: html.into() }
    }
}

/// A page provider. Return `None` to fall through (e.g. an override that only handles some
/// query strings and otherwise wants the built-in).
pub type PageProvider = Arc<dyn Fn(&PageRequest<'_>) -> Option<PageResponse> + Send + Sync>;

/// Registry of `gosub://` pages. Cheap to clone (shared); the engine holds one and hands
/// clones down to every tab.
#[derive(Clone, Default)]
pub struct InternalPages {
    inner: Arc<RwLock<Registry>>,
}

#[derive(Default)]
struct Registry {
    /// Embedder-registered providers, consulted first.
    overrides: HashMap<String, PageProvider>,
    /// Engine built-ins.
    builtin: HashMap<String, PageProvider>,
}

impl InternalPages {
    /// A registry seeded with the engine's built-in pages.
    pub fn with_builtins() -> Self {
        let pages = Self::default();
        {
            let mut reg = pages.inner.write();
            for (name, provider) in builtins::all() {
                reg.builtin.insert(name.to_string(), provider);
            }
        }
        pages
    }

    /// Register `provider` for `gosub://<name>`, overriding a built-in of the same name.
    pub fn register(&self, name: impl Into<String>, provider: PageProvider) {
        self.inner.write().overrides.insert(name.into(), provider);
    }

    /// Register a static HTML page.
    pub fn register_html(&self, name: impl Into<String>, html: impl Into<String>) {
        let html = html.into();
        self.register(name, Arc::new(move |_| Some(PageResponse::html(html.clone()))));
    }

    /// Remove an embedder override, restoring the built-in (if any).
    pub fn unregister(&self, name: &str) {
        self.inner.write().overrides.remove(name);
    }

    /// Names of every known page (overrides and built-ins), sorted.
    pub fn names(&self) -> Vec<String> {
        let reg = self.inner.read();
        let mut names: Vec<String> = reg.overrides.keys().chain(reg.builtin.keys()).cloned().collect();
        names.sort();
        names.dedup();
        names
    }

    /// Whether `url` is an internal-page URL this registry handles (`gosub:` or `about:`).
    pub fn handles(url: &Url) -> bool {
        matches!(url.scheme(), "gosub" | "about")
    }

    /// Page name of an internal URL: `gosub://blank` → `blank` (host form),
    /// `about:blank` → `blank` (opaque-path form). Empty for `gosub://` alone.
    pub fn page_name(url: &Url) -> &str {
        url.host_str().unwrap_or_else(|| url.path()).trim_matches('/')
    }

    /// Resolve `url` to a page. Overrides win over built-ins; a provider returning `None`
    /// falls through to the next candidate. Unknown pages get the built-in "no such page".
    pub fn resolve(&self, url: &Url, settings: &Config, tab: &TabView) -> PageResponse {
        let name = Self::page_name(url);
        let req = PageRequest {
            name,
            url,
            settings,
            tab,
        };
        let (over, built) = {
            let reg = self.inner.read();
            (reg.overrides.get(name).cloned(), reg.builtin.get(name).cloned())
        };
        for provider in [over, built].into_iter().flatten() {
            if let Some(resp) = provider(&req) {
                return resp;
            }
        }
        builtins::not_found(&req, &self.names())
    }
}

/// The engine's built-in pages. Deliberately unbranded and dependency-free (inline CSS,
/// system fonts): embedders override the ones they want to style.
pub mod builtins {
    use super::{PageProvider, PageRequest, PageResponse};
    use std::sync::Arc;

    pub(super) fn all() -> Vec<(&'static str, PageProvider)> {
        vec![
            ("blank", Arc::new(|_| Some(PageResponse::html(BLANK)))),
            ("home", Arc::new(|r| Some(home(r)))),
            ("help", Arc::new(|r| Some(help(r)))),
            ("version", Arc::new(|r| Some(version(r)))),
            ("history", Arc::new(|r| Some(history(r)))),
            ("config", Arc::new(|r| Some(config(r)))),
        ]
    }

    pub(super) fn escape(s: &str) -> String {
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

    /// Shared chrome for the built-in pages.
    fn page(title: &str, body: &str) -> PageResponse {
        PageResponse::html(format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title><style>\
             body{{margin:0;padding:32px 40px;font-family:sans-serif;font-size:14px;color:#1c2333;background:#ffffff}}\
             h1{{font-size:24px;margin:0 0 4px 0}} h2{{font-size:16px;margin:24px 0 8px 0}}\
             .sub{{color:#5c6675;margin:0 0 20px 0}}\
             table{{border-collapse:collapse}} td,th{{text-align:left;padding:3px 14px 3px 0;vertical-align:top}}\
             th{{color:#5c6675;font-weight:normal}} code{{font-family:monospace;font-size:13px}}\
             a{{color:#1d5fd1}} .muted{{color:#8a94a6}} .cur{{font-weight:bold}}\
             </style></head><body>{}</body></html>",
            escape(title),
            body
        ))
    }

    const BLANK: &str = "<!DOCTYPE html><html><head><title></title></head><body style=\"margin:0;background:#ffffff\"></body></html>";

    fn home(_r: &PageRequest<'_>) -> PageResponse {
        page(
            "New Tab",
            "<h1>Gosub</h1><p class=\"sub\">A new tab. See <a href=\"gosub://help\">gosub://help</a> for the internal pages.</p>",
        )
    }

    fn help(r: &PageRequest<'_>) -> PageResponse {
        let items = [
            ("home", "New-tab page"),
            ("blank", "Empty page (about:blank)"),
            ("help", "This page"),
            ("version", "Engine build, render backend, settings of note"),
            ("history", "This tab's session history tree"),
            ("config", "Engine settings (read-only dump)"),
        ];
        let mut body = String::from("<h1>Internal pages</h1><p class=\"sub\">Also reachable as <code>about:&lt;name&gt;</code>.</p><table>");
        for (name, desc) in items {
            body.push_str(&format!(
                "<tr><td><a href=\"gosub://{name}\"><code>gosub://{name}</code></a></td><td>{desc}</td></tr>"
            ));
        }
        body.push_str("</table>");
        let _ = r;
        page("Help", &body)
    }

    fn version(r: &PageRequest<'_>) -> PageResponse {
        let rows = [
            ("Engine", format!("gosub_engine {}", env!("CARGO_PKG_VERSION"))),
            ("Render backend", r.tab.render_backend.to_string()),
            ("User agent", r.settings.get_string("net.user_agent")),
            ("Target", format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH)),
        ];
        let mut body = String::from("<h1>Version</h1><table>");
        for (k, v) in rows {
            body.push_str(&format!("<tr><th>{}</th><td><code>{}</code></td></tr>", escape(k), escape(&v)));
        }
        body.push_str("</table>");
        page("Version", &body)
    }

    fn history(r: &PageRequest<'_>) -> PageResponse {
        let h = &r.tab.history;
        let mut body = String::from("<h1>Session history</h1><p class=\"sub\">This tab, oldest first. Bold is the current entry; the parent column shows the branch structure.</p>");
        if h.entries.is_empty() {
            body.push_str("<p class=\"muted\">No entries yet.</p>");
        } else {
            body.push_str("<table><tr><th>#</th><th>Parent</th><th>Title</th><th>URL</th></tr>");
            for e in &h.entries {
                let cls = if Some(e.id) == h.current { " class=\"cur\"" } else { "" };
                let title = e.title.as_deref().unwrap_or("");
                let parent = e.parent.map(|p| p.0.to_string()).unwrap_or_else(|| "–".to_string());
                body.push_str(&format!(
                    "<tr{cls}><td>{}</td><td>{parent}</td><td>{}</td><td><a href=\"{}\"><code>{}</code></a></td></tr>",
                    e.id.0,
                    escape(title),
                    escape(e.url.as_str()),
                    escape(e.url.as_str()),
                ));
            }
            body.push_str("</table>");
        }
        page("Session history", &body)
    }

    fn config(r: &PageRequest<'_>) -> PageResponse {
        let mut keys = r.settings.find("*");
        keys.sort();
        let mut body = String::from("<h1>Engine settings</h1><p class=\"sub\">Read-only dump of the settings store; bold rows differ from their default.</p><table><tr><th>Key</th><th>Value</th><th>Default</th><th>Description</th></tr>");
        for key in keys {
            let Some(info) = r.settings.get_info(&key) else { continue };
            let current = r.settings.get(&key).ok().flatten().unwrap_or_else(|| info.default.clone());
            let cls = if current != info.default { " class=\"cur\"" } else { "" };
            body.push_str(&format!(
                "<tr{cls}><td><code>{}</code></td><td><code>{}</code></td><td class=\"muted\"><code>{}</code></td><td>{}</td></tr>",
                escape(&key),
                escape(&current.value_string()),
                escape(&info.default.value_string()),
                escape(&info.description),
            ));
        }
        body.push_str("</table>");
        page("Engine settings", &body)
    }

    pub(super) fn not_found(r: &PageRequest<'_>, known: &[String]) -> PageResponse {
        let mut body = format!(
            "<h1>No such internal page</h1><p class=\"sub\"><code>{}</code> is not a page this engine knows.</p><h2>Available</h2><ul>",
            escape(r.url.as_str())
        );
        for name in known {
            body.push_str(&format!("<li><a href=\"gosub://{name}\"><code>gosub://{name}</code></a></li>"));
        }
        body.push_str("</ul>");
        page("No such page", &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::settings_store;

    fn resolve(pages: &InternalPages, url: &str) -> String {
        let url = Url::parse(url).unwrap();
        pages.resolve(&url, &settings_store::default_config(), &TabView::default()).html
    }

    #[test]
    fn page_name_handles_both_url_forms() {
        assert_eq!(InternalPages::page_name(&Url::parse("gosub://blank").unwrap()), "blank");
        assert_eq!(InternalPages::page_name(&Url::parse("about:blank").unwrap()), "blank");
        assert_eq!(InternalPages::page_name(&Url::parse("gosub://help/").unwrap()), "help");
        assert!(InternalPages::handles(&Url::parse("about:blank").unwrap()));
        assert!(!InternalPages::handles(&Url::parse("https://gosub.io").unwrap()));
    }

    #[test]
    fn builtins_resolve_and_unknown_gets_not_found() {
        let pages = InternalPages::with_builtins();
        assert!(resolve(&pages, "gosub://version").contains("gosub_engine"));
        assert!(resolve(&pages, "about:version").contains("gosub_engine"));
        assert!(resolve(&pages, "gosub://help").contains("gosub://history"));
        let nf = resolve(&pages, "gosub://nope");
        assert!(nf.contains("No such internal page"));
        assert!(nf.contains("gosub://nope"));
        assert!(nf.contains("gosub://blank"), "lists the known pages");
    }

    #[test]
    fn embedder_override_wins_and_can_fall_through() {
        let pages = InternalPages::with_builtins();
        pages.register_html("home", "<html><body>BRANDED</body></html>");
        assert!(resolve(&pages, "gosub://home").contains("BRANDED"));
        // A provider that declines falls through to the built-in.
        pages.register("version", Arc::new(|_| None));
        assert!(resolve(&pages, "gosub://version").contains("gosub_engine"));
        // Unregistering restores the built-in.
        pages.unregister("home");
        assert!(!resolve(&pages, "gosub://home").contains("BRANDED"));
        // New pages can be added.
        pages.register_html("mine", "<html><body>MINE</body></html>");
        assert!(resolve(&pages, "gosub://mine").contains("MINE"));
        assert!(pages.names().contains(&"mine".to_string()));
    }

    #[test]
    fn history_page_renders_the_tab_snapshot() {
        use crate::tab::history::History;
        let mut h = History::default();
        h.push(Url::parse("https://a.example/").unwrap(), Some("A".into()));
        h.push(Url::parse("https://b.example/").unwrap(), None);
        let tab = TabView {
            history: h.snapshot(),
            render_backend: "test",
        };
        let pages = InternalPages::with_builtins();
        let html = pages
            .resolve(&Url::parse("gosub://history").unwrap(), &settings_store::default_config(), &tab)
            .html;
        assert!(html.contains("https://a.example/"));
        assert!(html.contains("https://b.example/"));
        assert!(html.contains(">A<"));
    }

    #[test]
    fn user_content_is_escaped() {
        let pages = InternalPages::with_builtins();
        let html = resolve(&pages, "gosub://%3Cscript%3E");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;") || html.contains("%3Cscript%3E"));
    }
}
