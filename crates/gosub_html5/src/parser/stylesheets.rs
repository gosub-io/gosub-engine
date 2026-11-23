//! Stylesheet loading during the parse: inline `<style>` sheets, `<link>` sheets, their `@import`s,
//! and waiting for them before a blocking script.

use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;
use gosub_interface::css3::{CssOrigin, CssSystem};
use gosub_interface::document::{Document, PendingStylesheet};
use gosub_interface::node::NodeType;
use gosub_shared::attributes::Attributes;
use gosub_shared::config::{Context, ParserConfig};
use gosub_shared::node::NodeId;
use gosub_shared::timing::{self, Timer};
use log::warn;
use url::Url;

/// Whether a stylesheet loaded from `base` may import `resolved`.
///
/// `Url::join` returns the request unchanged when it carries its own scheme, so an `@import` can
/// name any URL it likes - including `file:`, which the fetch path would read straight off disk.
/// A document fetched over the network must never be able to reach the local filesystem that way,
/// so a remote sheet may only import over the network. A local sheet is left unrestricted: it can
/// already read the filesystem it came from.
#[cfg(not(target_arch = "wasm32"))]
fn import_scheme_allowed(base: Option<&Url>, resolved: &Url) -> bool {
    let base_is_remote = base.is_some_and(|u| u.scheme() != "file");
    !base_is_remote || matches!(resolved.scheme(), "http" | "https")
}

/// Somewhere the parser can get a linked stylesheet from, when it has to have one *now*.
///
/// The parser does not fetch, but a classic script may not run until the sheets before it
/// have applied, so it asks and blocks. That is only acceptable because the parse runs on the
/// blocking pool; on a runtime worker it would starve the fetches it waits for.
pub trait StylesheetSource: Send + Sync + std::fmt::Debug {
    /// Block until each URL has been fetched, and answer in the same order.
    ///
    /// `None` for a sheet that could not be had. A failure must come back as `None` rather
    /// than never returning: a page whose stylesheet 404s still has to finish parsing.
    fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>>;
}

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Records the tree scope a freshly parsed stylesheet belongs to, then hands it to the
    /// document.
    ///
    /// `owner` is the `<style>` or `<link>` element the sheet came from. Its scope is the shadow
    /// root at the top of its ancestor chain, or `None` when that chain reaches the document -
    /// a shadow root has no parent, so the walk stops there by itself.
    pub(crate) fn add_stylesheet_for(
        &mut self,
        owner: NodeId,
        mut stylesheet: <C::CssSystem as CssSystem>::Stylesheet,
    ) {
        let mut root = owner;
        while let Some(parent) = self.document.parent(root) {
            root = parent;
        }
        let scope = (self.document.node_type(root) == NodeType::ShadowRootNode).then_some(root);

        C::CssSystem::set_stylesheet_scope(&mut stylesheet, scope);
        self.document.add_stylesheet(stylesheet);
    }

    /// Whether `id` is in the contents of an ordinary `<template>`, which belong to a document
    /// of their own. A declarative shadow root's template is not in the tree, so a shadow tree
    /// is only in template contents when its host is.
    pub(crate) fn in_template_contents(&self, id: NodeId) -> bool {
        let mut node = Some(id);
        while let Some(id) = node {
            if self.document.template_contents(id) == Some(id) {
                return true;
            }
            node = self.document.parent(id).or_else(|| self.document.shadow_host(id));
        }
        false
    }

    /// Wait for every stylesheet linked so far, and put it in the cascade.
    ///
    /// Called before a script that blocks. Everything still pending is a `<link>` before this
    /// script, since a later one has not been tokenised yet, so all of them block it.
    ///
    /// Does nothing without a [`StylesheetSource`]: the sheets stay pending and whoever
    /// drives the parse resolves them afterwards, which is what happens for a document with
    /// no blocking scripts in it.
    pub(crate) fn settle_stylesheets(&mut self) {
        let Some(source) = self.stylesheet_source.clone() else {
            return;
        };
        let pending = self.document.take_pending_stylesheets();
        if pending.is_empty() {
            return;
        }

        let urls: Vec<String> = pending.iter().map(|sheet| sheet.url.clone()).collect();
        let mut waited = Timer::new(Some(format!("{} stylesheet(s)", urls.len())));
        let bodies = source.fetch_blocking(&urls);
        waited.end();

        // What a script waiting on CSS actually cost, which is the number worth having when
        // a page loads slowly and nobody can say why.
        timing::record(
            gosub_shared::timing::Timing::ScriptBlockedOnCss,
            waited.duration(),
            Some(urls.join(" ")),
        );
        self.blocked_on_css_us
            .set(self.blocked_on_css_us.get() + waited.duration());
        log::debug!(
            "script blocked on {} stylesheet(s) for {:.1}ms",
            urls.len(),
            waited.duration() as f64 / 1000.0
        );

        let mut inserted = 0usize;
        for (PendingStylesheet { position, url, media }, body) in pending.into_iter().zip(bodies) {
            let Some(bytes) = body else {
                warn!("Could not load external stylesheet from {url}");
                continue;
            };
            let Ok(css) = String::from_utf8(bytes) else {
                warn!("External stylesheet from {url} is not valid UTF-8");
                continue;
            };
            let config = ParserConfig {
                source: Some(url.clone()),
                ignore_errors: true,
                ..Default::default()
            };
            match C::CssSystem::parse_str(&css, config, CssOrigin::Author, &url) {
                // Each sheet already placed shifts this one along by one; see
                // `Document::add_pending_stylesheet` for what the position means.
                Ok(mut sheet) => {
                    if let Some(media) = &media {
                        C::CssSystem::set_stylesheet_media(&mut sheet, media);
                    }
                    self.document.insert_stylesheet(position + inserted, sheet);
                    inserted += 1;
                }
                Err(err) => warn!("Error while parsing CSS stylesheet from {url}: {err}"),
            }
        }
    }

    /// Load an inline stylesheet from the <style>-node (identified by NodeId)
    pub(crate) fn load_inline_stylesheet(
        &self,
        origin: CssOrigin,
        node_id: NodeId,
    ) -> Option<<C::CssSystem as CssSystem>::Stylesheet> {
        // Must be a text node
        let text = self.document.text_value(node_id)?;

        let source_url = match self.document.url() {
            Some(url) => format!("{url}#inline"),
            None => "<unknown>#inline".into(),
        };

        let config = ParserConfig {
            context: Context::Stylesheet,
            location: Default::default(),
            source: Some(source_url.clone()),
            ignore_errors: true,
            match_values: false,
        };

        match C::CssSystem::parse_str(text, config, origin, &source_url) {
            Ok(mut stylesheet) => {
                self.resolve_stylesheet_imports(&mut stylesheet);
                Some(stylesheet)
            }
            Err(err) => {
                warn!("Error while parsing CSS stylesheet: {err} ");
                None
            }
        }
    }

    /// The text of a stylesheet an `@import` asked for.
    ///
    /// Asks the [`StylesheetSource`], the same one a `<link>` goes through, so an import is
    /// fetched under the same policy as any other subresource. Without a source the import
    /// stays unresolved.
    ///
    /// Blocks, like [`Self::settle_stylesheets`]. The wait is added to `blocked_on_css_us` so
    /// `parse_document` can take it out of `decode.html`.
    pub(crate) fn fetch_css_text(&self, url: &Url) -> Option<String> {
        let source = self.stylesheet_source.clone()?;

        let mut waited = Timer::new(Some(url.to_string()));
        let body = source.fetch_blocking(&[url.to_string()]).pop().flatten();
        waited.end();
        self.blocked_on_css_us
            .set(self.blocked_on_css_us.get() + waited.duration());

        match body {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(css) => Some(css),
                Err(err) => {
                    warn!("Imported stylesheet from {url} is not valid UTF-8: {err}");
                    None
                }
            },
            None => {
                warn!("Could not load imported stylesheet from {url}");
                None
            }
        }
    }

    /// Pull in every `@import` the stylesheet declares.
    ///
    /// Each import is another round trip the parse waits on, and they are sequential, so a
    /// chain costs its full depth in latency. Handing imports to the resource pipeline the
    /// way `<link>` is handed to it is the fix; until then an unresolved `@import` silently
    /// loses a whole stylesheet, which is worse.
    pub(crate) fn resolve_stylesheet_imports(&self, sheet: &mut <C::CssSystem as CssSystem>::Stylesheet) {
        let mut fetch = |base: &str, requested: &str| {
            // A relative import resolves against the importing sheet, not the document.
            let base_url = Url::parse(base).ok();
            let resolved = base_url
                .as_ref()
                .and_then(|base| base.join(requested).ok())
                .or_else(|| Url::parse(requested).ok())?;

            if !import_scheme_allowed(base_url.as_ref(), &resolved) {
                warn!(
                    "Refusing '{}' import from remote stylesheet {}: only http(s) is allowed",
                    resolved.scheme(),
                    base
                );
                return None;
            }

            let text = self.fetch_css_text(&resolved)?;
            Some((resolved.to_string(), text))
        };
        C::CssSystem::resolve_imports(sheet, &mut fetch);
    }

    /// What a `<link>` asks of the document: today only a stylesheet does anything. What it
    /// does not do is logged, not reported as a parse error: none of it is one (the tree
    /// builder's rules for `<link>` have no error case), and a page's report should hold
    /// only what the HTML spec calls an error.
    pub(crate) fn handle_link_element(&mut self, attributes: &Attributes) {
        if attributes.contains_key("rel") && attributes.contains_key("itemprop") {
            // cannot have them both
            log::debug!("link element cannot have both 'rel' and 'itemprop' attributes");
            return;
        }

        if attributes.contains_key("itemprop") {
            log::debug!("link element with 'itemprop' attribute not supported yet");
            return;
        }

        let Some(rel) = attributes.get("rel").cloned() else {
            log::debug!("link element without 'rel' attribute not supported yet");
            return;
        };

        // @todo: We need to check if the link rel is body-ok
        let parser_in_body = true;
        let body_ok_types = [
            "dns-prefetch",
            "modulepreload",
            "pingback",
            "preconnect",
            "prefetch",
            "preload",
            "prerender",
            "stylesheet",
        ];
        if parser_in_body && !body_ok_types.contains(&rel.as_str()) {
            log::debug!("link element with rel attribute '{rel}' is not supported in the body");
            return;
        }

        match rel.as_str() {
            "stylesheet" => {
                // Stylesheet link may not have a href link (for instance, it could be data-href for optional loading through JS)
                let Some(href) = attributes.get("href") else {
                    return;
                };

                let css_url = match Url::parse(href) {
                    Ok(url) => url,
                    Err(_err) => {
                        // Relative URL
                        let Some(base_url) = self.document.url() else {
                            log::debug!("link element without base url not supported yet");
                            return;
                        };
                        match base_url.join(href) {
                            Ok(url) => url,
                            Err(_) => {
                                log::debug!("link element with invalid href url");
                                return;
                            }
                        }
                    }
                };
                // Recorded, not fetched. The parser has no business opening sockets: the
                // resource pipeline is already fetching this sheet (and everything else the
                // document links), and it fills the slot in once the bytes are here.
                let media = attributes.get("media").map(String::as_str);
                self.document.add_pending_stylesheet(css_url.as_str(), media);
            }
            _ => {
                log::debug!("link element with rel attribute '{rel}' is not supported");
            }
        }
    }
}

#[cfg(test)]
mod import_scheme_tests {
    use super::import_scheme_allowed;
    use url::Url;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The one that matters: a sheet served over the network must not be able to name a local
    /// file. `Url::join` hands back the absolute `file:` URL, and the fetch path would read it.
    #[test]
    fn a_remote_sheet_cannot_import_a_local_file() {
        let base = url("https://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));

        let base = url("http://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));
    }

    #[test]
    fn a_remote_sheet_may_import_over_the_network() {
        let base = url("https://example.org/a.css");
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
        assert!(import_scheme_allowed(Some(&base), &url("http://cdn.example.org/b.css")));
    }

    /// A local sheet keeps working: it can already read the filesystem it came from.
    #[test]
    fn a_local_sheet_is_unrestricted() {
        let base = url("file:///home/u/site/a.css");
        assert!(import_scheme_allowed(Some(&base), &url("file:///home/u/site/b.css")));
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
    }

    /// Remote is anything not read from disk: a page served over gopher, say.
    #[test]
    fn a_sheet_from_any_other_scheme_is_remote() {
        let base = url("gopher://example.org/0/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("file:///etc/passwd")));
        assert!(import_scheme_allowed(
            Some(&base),
            &url("https://cdn.example.org/b.css")
        ));
    }

    /// An unparseable base is not known to be remote, so it is not treated as one.
    #[test]
    fn an_unknown_base_is_not_treated_as_remote() {
        assert!(import_scheme_allowed(None, &url("file:///etc/passwd")));
    }

    /// Other non-network schemes are refused from a remote sheet too, not just `file:`.
    #[test]
    fn other_non_network_schemes_are_refused_from_remote() {
        let base = url("https://example.org/a.css");
        assert!(!import_scheme_allowed(Some(&base), &url("data:text/css,body{}")));
    }
}

#[cfg(test)]
mod owner_media_tests {
    use crate::document::document_impl::DocumentImpl;
    use crate::parser::Html5Parser;
    use gosub_css3::media_query::MediaEnvironment;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::{Document, PendingStylesheet};

    #[derive(Clone, Debug, PartialEq)]
    struct Config;
    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    #[test]
    fn a_style_element_keeps_its_media_attribute() {
        let doc = crate::html_compile::<Config>(
            "<head><style media=print>p { color: red; }</style><style>p { color: blue; }</style></head>",
        );
        let screen = MediaEnvironment::default();
        let [print, plain] = doc.stylesheets() else {
            panic!("two sheets");
        };
        assert!(!print.rules[0].media_matches(&screen));
        assert!(plain.rules[0].media.is_none());
    }

    #[test]
    fn a_link_hands_its_media_attribute_to_whoever_fetches_it() {
        let mut doc = crate::html_compile::<Config>(
            r#"<head><base href="https://example.org/">
               <link rel=stylesheet href="https://example.org/p.css" media="print">
               <link rel=stylesheet href="https://example.org/a.css"></head>"#,
        );
        let pending = doc.take_pending_stylesheets();
        let media: Vec<_> = pending
            .iter()
            .map(|PendingStylesheet { media, .. }| media.as_deref())
            .collect();
        assert_eq!(media, [Some("print"), None]);
    }
}
