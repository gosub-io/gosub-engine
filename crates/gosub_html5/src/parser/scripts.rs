//! Running the document's classic scripts during the parse: inline and blocking ones where
//! they stand, `defer` and `async` ones when the document has been parsed.

use crate::node::{HTML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::Html5Parser;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_interface::script::ScriptOrigin;
use gosub_shared::node::NodeId;
use gosub_shared::timing::Timer;
use log::warn;
use url::Url;
/// A script element the parser runs.
enum Script {
    Inline(String),
    External { url: String, when: ScriptTiming },
}

/// When an external script runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptTiming {
    /// Where it stands; the parse waits for it.
    Blocking,
    /// After the parse, once the stylesheets are in.
    Defer,
    /// After the parse for now, and without waiting for stylesheets.
    Async,
}

/// The JavaScript MIME type essences.
const JAVASCRIPT_MIME_TYPES: [&str; 16] = [
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];

/// Whether a script element with these `type` and `language` attributes is a classic
/// script: no type, or a JavaScript MIME type. A module or a data block is not.
fn is_classic_script_type(type_attr: Option<&str>, language: Option<&str>) -> bool {
    let is_javascript = |mime: &str| JAVASCRIPT_MIME_TYPES.iter().any(|js| mime.eq_ignore_ascii_case(js));

    match (type_attr, language) {
        (Some(""), _) | (None, None | Some("")) => true,
        // a type is matched after trimming, a language as it is written
        (Some(type_attr), _) => is_javascript(type_attr.trim_matches(|c: char| c.is_ascii_whitespace())),
        (None, Some(language)) => is_javascript(&format!("text/{language}")),
    }
}

/// Somewhere the parser can get an external script from, when it is time to run it.
///
/// Like [`StylesheetSource`]: the parser does not fetch, it asks and blocks.
pub trait ScriptSource: Send + Sync + std::fmt::Debug {
    /// Block until the script at `url` has been fetched, and answer with its text.
    ///
    /// `None` for a script that could not be had; it must not hang instead.
    fn fetch_blocking(&self, url: &str) -> Option<String>;
}

impl<C: HasDocument> Html5Parser<'_, C> {
    /// Runs the script element whose end tag was just seen, if it is one this parser runs
    /// now: a classic script, inline or blocking, in a document whose scripts run. A `defer`
    /// or `async` script is kept for the end of the parse. Returns whether a script ran.
    ///
    /// A script that fails does not stop the parse; the host reports its own errors.
    pub(crate) fn run_script(&mut self, script: NodeId) -> bool {
        if self.script_host.is_none() || !self.scripting_enabled || self.is_fragment_case {
            return false;
        }
        // the contents of a template belong to a document that runs nothing
        let in_template =
            |id: &NodeId| node_tag!(self, *id) == "template" && node_ns!(self, *id) == Some(HTML_NAMESPACE);
        if self.open_elements.iter().any(in_template) {
            return false;
        }

        let mut spent = Timer::new(None);
        let ran = self.run_classic_script(script);
        spent.end();
        self.script_us += spent.duration();

        ran
    }

    fn run_classic_script(&mut self, script: NodeId) -> bool {
        match self.classic_script(script) {
            Some(Script::Inline(source)) => {
                // The tokenizer is just past the end tag; the source starts as many lines
                // up as it has line breaks.
                let end_line = u32::try_from(self.tokenizer.get_location().line).unwrap_or(0);
                let line_breaks = u32::try_from(source.matches('\n').count()).unwrap_or(0);
                let origin = ScriptOrigin {
                    url: self.document.url().map(|url| url.to_string()).unwrap_or_default(),
                    line: end_line.saturating_sub(1).saturating_sub(line_breaks),
                };
                self.execute_script(&source, &origin)
            }
            Some(Script::External {
                url,
                when: ScriptTiming::Blocking,
            }) => self.run_external_script(&url),
            Some(Script::External { url, when }) => {
                self.deferred_scripts.push((url, when));
                false
            }
            None => false,
        }
    }

    /// Fetches and runs an external script. One that cannot be loaded is skipped.
    fn run_external_script(&mut self, url: &str) -> bool {
        let Some(source) = self
            .script_source
            .as_ref()
            .and_then(|source| source.fetch_blocking(url))
        else {
            warn!("Could not load script from {url}");
            return false;
        };

        let origin = ScriptOrigin {
            url: url.to_owned(),
            line: 0,
        };
        self.execute_script(&source, &origin)
    }

    fn execute_script(&mut self, source: &str, origin: &ScriptOrigin) -> bool {
        let Some(host) = self.script_host.as_mut() else {
            return false;
        };
        if let Err(e) = host.execute(source, origin) {
            log::debug!("script in {} failed: {e}", origin.url);
        }

        true
    }

    /// Runs the `defer` and `async` scripts, in document order, now that the document has
    /// been parsed. `async` scripts do not run earlier than this yet.
    pub(crate) fn run_deferred_scripts(&mut self) {
        let mut settled = false;
        for (url, when) in std::mem::take(&mut self.deferred_scripts) {
            // a `defer` script waits for the stylesheets of the document, as a blocking
            // script does; an `async` one does not
            if when == ScriptTiming::Defer && !settled {
                self.settle_stylesheets();
                settled = true;
            }
            if self.run_external_script(&url) {
                self.script_checkpoint();
            }
        }
    }

    /// The promise jobs a script queued run once no script is on the stack.
    pub(crate) fn script_checkpoint(&mut self) {
        if let Some(host) = self.script_host.as_mut() {
            if let Err(e) = host.microtask_checkpoint() {
                log::debug!("promise job failed: {e}");
            }
        }
    }

    /// What there is to run for a script element: `None` when it is not a classic script,
    /// has nothing in it, or is external with nowhere to get it from.
    fn classic_script(&self, script: NodeId) -> Option<Script> {
        let attributes = self.document.attributes(script);
        let attribute = |name: &str| attributes.and_then(|a| a.get(name)).map(String::as_str);

        if !is_classic_script_type(attribute("type"), attribute("language")) {
            return None;
        }
        // an SVG script names its file with `href`; those are not loaded yet
        let in_svg = node_ns!(self, script) == Some(SVG_NAMESPACE);
        // (`xlink:href` is stored under its adjusted name)
        if in_svg && (attribute("href").is_some() || attribute("xlink href").is_some()) {
            return None;
        }

        if let Some(src) = attribute("src").filter(|_| !in_svg) {
            self.script_source.as_ref()?;
            if src.is_empty() {
                return None;
            }
            // resolved the way a stylesheet link is
            let url = match Url::parse(src) {
                Ok(url) => url,
                Err(_) => self.document.url()?.join(src).ok()?,
            };

            return Some(Script::External {
                url: url.to_string(),
                when: if attribute("async").is_some() {
                    ScriptTiming::Async
                } else if attribute("defer").is_some() {
                    ScriptTiming::Defer
                } else {
                    ScriptTiming::Blocking
                },
            });
        }

        let source: String = self
            .document
            .children(script)
            .iter()
            .filter_map(|&child| self.document.text_value(child))
            .collect();

        (!source.is_empty()).then_some(Script::Inline(source))
    }
}

#[cfg(test)]
mod script_tests {
    use std::sync::Arc;

    use super::*;
    use crate::document::builder::DocumentBuilderImpl;
    use crate::document::document_impl::DocumentImpl;
    use crate::parser::{Html5ParserOptions, StylesheetSource};
    use gosub_css3::system::Css3System;
    use gosub_interface::config::HasCssSystem;
    use gosub_interface::script::ScriptHost;
    use gosub_interface::script::{ScriptError, ScriptErrorKind, ScriptTerminator, ScriptValue};
    use gosub_shared::attributes::Attributes;
    use gosub_shared::byte_stream::{ByteStream, Encoding, Location};
    use parking_lot::Mutex;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl HasCssSystem for Config {
        type CssSystem = Css3System;
    }
    impl HasDocument for Config {
        type Document = DocumentImpl<Self>;
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Call {
        Execute {
            source: String,
            url: String,
            line: u32,
        },
        Checkpoint,
        /// The stylesheets the parser asked for.
        Sheets(Vec<String>),
    }

    /// Records what the parser asks of it. A source that starts with `throw` fails.
    struct Recorder(Arc<Mutex<Vec<Call>>>);

    impl ScriptHost for Recorder {
        fn execute(&mut self, source: &str, origin: &ScriptOrigin) -> std::result::Result<ScriptValue, ScriptError> {
            self.0.lock().push(Call::Execute {
                source: source.to_owned(),
                url: origin.url.clone(),
                line: origin.line,
            });
            if source.starts_with("throw") {
                return Err(ScriptError::new(ScriptErrorKind::Exception, "thrown"));
            }
            Ok(ScriptValue::Undefined)
        }

        fn microtask_checkpoint(&mut self) -> std::result::Result<(), ScriptError> {
            self.0.lock().push(Call::Checkpoint);
            Ok(())
        }

        fn terminator(&self) -> ScriptTerminator {
            ScriptTerminator::noop()
        }
    }

    fn calls_with(html: &str, scripting_enabled: bool) -> Vec<Call> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: None,
        };

        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let url = Url::parse("https://example.test/page.html").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let calls = calls.lock().clone();
        calls
    }

    /// The sources the parser ran for `html`, in order.
    fn sources(html: &str) -> Vec<String> {
        calls_with(html, true)
            .into_iter()
            .filter_map(|call| match call {
                Call::Execute { source, .. } => Some(source),
                _ => None,
            })
            .collect()
    }

    fn execute(source: &str, line: u32) -> Call {
        Call::Execute {
            source: source.to_owned(),
            url: "https://example.test/page.html".to_owned(),
            line,
        }
    }

    #[test]
    fn inline_scripts_run_in_document_order_each_followed_by_a_checkpoint() {
        let html = "<html><head><script>first()</script></head><body><p>x</p><script>second()</script></body></html>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("first()", 0),
                Call::Checkpoint,
                execute("second()", 0),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn a_failing_script_does_not_stop_the_next() {
        let html = "<script>throw 1</script><script>after()</script>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("throw 1", 0),
                Call::Checkpoint,
                execute("after()", 0),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn the_origin_has_the_line_the_source_starts_on() {
        let html = "<html>\n<body>\n<script>\na()\nb()\n</script>\n<script>c()</script>";

        assert_eq!(
            calls_with(html, true),
            [
                execute("\na()\nb()\n", 2),
                Call::Checkpoint,
                execute("c()", 6),
                Call::Checkpoint
            ]
        );
    }

    #[test]
    fn a_script_in_svg_runs() {
        assert_eq!(sources("<svg><script>inSvg()</script></svg>"), ["inSvg()"]);
        assert_eq!(
            sources("<svg><script href=\"a.js\">no()</script></svg>"),
            [] as [&str; 0]
        );
        assert_eq!(
            sources("<svg><script type=\"text/plain\">no()</script></svg>"),
            [] as [&str; 0]
        );
    }

    #[test]
    fn only_javascript_types_run() {
        let run = |attrs: &str| !sources(&format!("<script {attrs}>x()</script>")).is_empty();

        for attrs in [
            "",
            "type=\"\"",
            "type=\"text/javascript\"",
            "type=\" TEXT/JavaScript \"",
            "type=\"application/javascript\"",
            "type=\"application/ecmascript\"",
            "type=\"text/jscript\"",
            "language=\"\"",
            "language=\"JavaScript\"",
            "language=\"javascript1.5\"",
            // the type decides when both are there
            "type=\"text/javascript\" language=\"vbscript\"",
            // no module support, so nomodule scripts are for this parser
            "nomodule",
        ] {
            assert!(run(attrs), "<script {attrs}> did not run");
        }

        for attrs in [
            "type=\"module\"",
            "type=\"text/plain\"",
            "type=\"application/json\"",
            "type=\"importmap\"",
            "type=\"text/javascript; charset=utf-8\"",
            "type=\"javascript\"",
            "language=\"vbscript\"",
            "language=\" javascript\"",
            "type=\"text/plain\" language=\"javascript\"",
        ] {
            assert!(!run(attrs), "<script {attrs}> ran");
        }
    }

    #[test]
    fn external_and_empty_scripts_are_not_run_here() {
        assert_eq!(sources("<script src=\"a.js\">inline()</script>"), [] as [&str; 0]);
        assert_eq!(sources("<script src=\"\">inline()</script>"), [] as [&str; 0]);
        assert_eq!(sources("<script></script>"), [] as [&str; 0]);
    }

    #[test]
    fn a_script_in_a_template_does_not_run() {
        assert_eq!(
            sources("<template><script>inTemplate()</script></template><script>outside()</script>"),
            ["outside()"]
        );
        assert_eq!(
            sources("<template><div><script>nested()</script></div></template>"),
            [] as [&str; 0]
        );
    }

    #[test]
    fn nothing_runs_with_scripting_disabled() {
        assert_eq!(calls_with("<script>x()</script>", false), []);
    }

    #[test]
    fn nothing_runs_in_a_fragment() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: None,
        };

        // the way innerHTML parses: a fresh document, with the context element made in it
        let mut doc = DocumentBuilderImpl::new_document::<Config>(None);
        let context = doc.create_element("div", Some(HTML_NAMESPACE), Attributes::new(), Location::default());
        let mut fragment = ByteStream::from_str("<script>inFragment()</script>", Encoding::UTF8);
        let _ =
            Html5Parser::<Config>::parse_fragment(&mut fragment, &mut doc, context, Some(options), Location::default());

        assert_eq!(*calls.lock(), []);
    }

    #[test]
    fn script_text_is_taken_as_written() {
        assert_eq!(
            sources("<script>if (a < b && c) { d('</p>') } // <!-- x</script>"),
            ["if (a < b && c) { d('</p>') } // <!-- x"]
        );
    }

    /// Serves `/<name>.js` as `<name>()`, except `missing.js`. Records what was asked for.
    #[derive(Debug, Default)]
    struct Files(Mutex<Vec<String>>);

    impl ScriptSource for Files {
        fn fetch_blocking(&self, url: &str) -> Option<String> {
            self.0.lock().push(url.to_owned());
            let name = url.strip_prefix("https://example.test/")?.strip_suffix(".js")?;

            (name != "missing").then(|| format!("{name}()"))
        }
    }

    /// The scripts the parser ran for `html` as (source, url), and the URLs it fetched.
    fn external(html: &str) -> (Vec<(String, String)>, Vec<String>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let files = Arc::new(Files::default());
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::clone(&files) as Arc<dyn ScriptSource>),
        };

        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let url = Url::parse("https://example.test/dir/page.html").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let ran = calls
            .lock()
            .iter()
            .filter_map(|call| match call {
                Call::Execute { source, url, line } => {
                    assert!(
                        *line == 0 || !url.ends_with(".js"),
                        "an external script starts on line 0"
                    );
                    Some((source.clone(), url.clone()))
                }
                _ => None,
            })
            .collect();
        let fetched = files.0.lock().clone();

        (ran, fetched)
    }

    fn ran(html: &str) -> Vec<String> {
        external(html).0.into_iter().map(|(source, _)| source).collect()
    }

    #[test]
    fn a_blocking_external_script_runs_where_it_stands() {
        let (ran, fetched) =
            external("<script>before()</script><script src=\"/a.js\"></script><script>after()</script>");

        assert_eq!(
            ran,
            [
                ("before()".to_owned(), "https://example.test/dir/page.html".to_owned()),
                ("a()".to_owned(), "https://example.test/a.js".to_owned()),
                ("after()".to_owned(), "https://example.test/dir/page.html".to_owned()),
            ]
        );
        assert_eq!(fetched, ["https://example.test/a.js"]);
    }

    #[test]
    fn a_src_is_resolved_against_the_document() {
        let (_, fetched) = external(
            "<script src=\"rel.js\"></script><script src=\"../up.js\"></script>\
             <script src=\"https://example.test/abs.js\"></script>",
        );

        assert_eq!(
            fetched,
            [
                "https://example.test/dir/rel.js",
                "https://example.test/up.js",
                "https://example.test/abs.js"
            ]
        );
    }

    #[test]
    fn defer_and_async_scripts_run_after_the_parse_in_document_order() {
        let html = "<script defer src=\"/d1.js\"></script><script>inline1()</script>\
                    <script async src=\"/a1.js\"></script><script src=\"/block.js\"></script>\
                    <script defer src=\"/d2.js\"></script><script>inline2()</script>";

        assert_eq!(ran(html), ["inline1()", "block()", "inline2()", "d1()", "a1()", "d2()"]);
    }

    #[test]
    fn each_script_is_followed_by_a_checkpoint() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: None,
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::new(Files::default())),
        };
        let mut stream = ByteStream::from_str(
            "<script src=\"/a.js\"></script><script defer src=\"/d.js\"></script>",
            Encoding::UTF8,
        );
        let url = Url::parse("https://example.test/").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let kinds: Vec<bool> = calls.lock().iter().map(|call| *call == Call::Checkpoint).collect();
        assert_eq!(kinds, [false, true, false, true]);
    }

    #[test]
    fn a_script_that_cannot_be_loaded_is_skipped() {
        assert_eq!(
            ran(
                "<script src=\"/missing.js\"></script><script defer src=\"/missing.js\"></script>\
                 <script src=\"/a.js\"></script><script defer src=\"/d.js\"></script>"
            ),
            ["a()", "d()"]
        );
    }

    #[test]
    fn the_text_of_an_external_script_element_is_not_run() {
        assert_eq!(ran("<script src=\"/a.js\">inline()</script>"), ["a()"]);
        assert_eq!(ran("<script src=\"/missing.js\">inline()</script>"), [] as [&str; 0]);
    }

    #[test]
    fn external_scripts_that_are_not_fetched() {
        for html in [
            "<script src=\"\"></script>",
            "<script type=\"module\" src=\"/a.js\"></script>",
            "<script type=\"text/plain\" src=\"/a.js\"></script>",
            "<template><script src=\"/a.js\"></script></template>",
            "<svg><script href=\"/a.js\"></script></svg>",
        ] {
            let (ran, fetched) = external(html);
            assert!(
                ran.is_empty() && fetched.is_empty(),
                "{html}: ran {ran:?}, fetched {fetched:?}"
            );
        }
    }

    #[test]
    fn href_only_names_a_file_on_an_svg_script() {
        assert_eq!(ran("<script href=\"/a.js\">inline()</script>"), ["inline()"]);
        assert_eq!(
            ran("<svg><script href=\"/a.js\">inline()</script></svg>"),
            [] as [&str; 0]
        );
        // an SVG `template` is not a template
        assert_eq!(
            ran("<svg><template><script>inSvg()</script></template></svg>"),
            ["inSvg()"]
        );
        // and `src` only on an HTML one
        assert_eq!(ran("<svg><script src=\"/a.js\">inSvg()</script></svg>"), ["inSvg()"]);
    }

    #[test]
    fn xlink_href_names_a_file_on_an_svg_script() {
        for html in [
            "<svg><script xlink:href=\"/a.js\">inline()</script></svg>",
            "<svg><script src=\"/ignored.js\" xlink:href=\"/a.js\">inline()</script></svg>",
        ] {
            let (ran, fetched) = external(html);
            assert!(
                ran.is_empty() && fetched.is_empty(),
                "{html}: ran {ran:?}, fetched {fetched:?}"
            );
        }
        // not an attribute of an HTML script
        assert_eq!(ran("<script xlink:href=\"/a.js\">inline()</script>"), ["inline()"]);
    }

    /// Records the stylesheets asked for in the same log as the scripts, and has none.
    #[derive(Debug)]
    struct Sheets(Arc<Mutex<Vec<Call>>>);

    impl StylesheetSource for Sheets {
        fn fetch_blocking(&self, urls: &[String]) -> Vec<Option<Vec<u8>>> {
            self.0.lock().push(Call::Sheets(urls.to_vec()));
            vec![None; urls.len()]
        }
    }

    #[test]
    fn deferred_scripts_wait_for_the_stylesheets() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: Some(Arc::new(Sheets(Arc::clone(&calls)))),
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::new(Files::default())),
        };
        let mut stream = ByteStream::from_str(
            "<script defer src=\"/d.js\"></script><link rel=\"stylesheet\" href=\"/late.css\">",
            Encoding::UTF8,
        );
        let url = Url::parse("https://example.test/").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        assert_eq!(
            *calls.lock(),
            [
                Call::Sheets(vec!["https://example.test/late.css".to_owned()]),
                Call::Execute {
                    source: "d()".to_owned(),
                    url: "https://example.test/d.js".to_owned(),
                    line: 0
                },
                Call::Checkpoint,
            ]
        );
    }

    /// What a parse with pending stylesheets asks for and runs: `sheets` or a script's source.
    fn with_late_sheet(html: &str) -> Vec<String> {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let options = Html5ParserOptions {
            scripting_enabled: true,
            stylesheets: Some(Arc::new(Sheets(Arc::clone(&calls)))),
            scripts: Some(Box::new(Recorder(Arc::clone(&calls)))),
            script_sources: Some(Arc::new(Files::default())),
        };
        let mut stream = ByteStream::from_str(html, Encoding::UTF8);
        let url = Url::parse("https://example.test/").unwrap();
        let mut doc = DocumentBuilderImpl::new_document::<Config>(Some(url));
        let _ = Html5Parser::<Config>::parse_document(&mut stream, &mut doc, Some(options));

        let calls = calls.lock();
        calls
            .iter()
            .filter_map(|call| match call {
                Call::Sheets(_) => Some("sheets".to_owned()),
                Call::Execute { source, .. } => Some(source.clone()),
                Call::Checkpoint => None,
            })
            .collect()
    }

    #[test]
    fn async_scripts_do_not_wait_for_the_stylesheets() {
        const LINK: &str = "<link rel=\"stylesheet\" href=\"/late.css\">";

        assert_eq!(
            with_late_sheet(&format!("<script async src=\"/a1.js\"></script>{LINK}")),
            ["a1()"]
        );
        // `async` wins over `defer`
        assert_eq!(
            with_late_sheet(&format!("<script async defer src=\"/a1.js\"></script>{LINK}")),
            ["a1()"]
        );
        // only the deferred ones wait, and once
        assert_eq!(
            with_late_sheet(&format!(
                "<script async src=\"/a1.js\"></script><script defer src=\"/d1.js\"></script>\
                 <script async src=\"/a.js\"></script><script defer src=\"/d2.js\"></script>{LINK}"
            )),
            ["a1()", "sheets", "d1()", "a()", "d2()"]
        );
    }

    #[test]
    fn defer_on_an_inline_script_changes_nothing() {
        assert_eq!(
            ran("<script defer>first()</script><script async>second()</script><script src=\"/a.js\"></script>"),
            ["first()", "second()", "a()"]
        );
    }
}
