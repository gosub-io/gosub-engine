//! Style resolution benchmark: what it costs to give every element its computed style.
//!
//! Run: cargo bench -p gosub_render_pipeline --bench style
//!
//! Record a baseline before changing the style system, then compare against it after:
//!
//!   cargo bench -p gosub_render_pipeline --bench style -- --save-baseline before
//!   cargo bench -p gosub_render_pipeline --bench style -- --baseline before
//!
//! Two measurements per fixture, so a change can be attributed to the CSS crate or to the
//! render pipeline's use of it:
//!
//! - `cascade`: the CSS crate alone. Walk the element tree top-down and ask `Css3System` for
//!   each element's property map, given the parent's, then compute every property. This is
//!   exactly what the pipeline adapter does per node, minus the adapter.
//! - `render-tree`: stage 1 of the pipeline, a fresh adapter plus `RenderTree::parse()`. It
//!   styles every node through the adapter's cache and reads `display` back through the typed
//!   conversion, so it also covers the string-to-typed bridge.
//!
//! Parsing the HTML and the stylesheets happens once, outside the measured loop: this measures
//! styling, not parsing, and the media environment is pinned to a 1280x800 viewport.
//!
//! Fixtures:
//!
//! - `small`: a dozen elements and the user-agent sheet. The floor.
//! - `utility-3k`: a generated page of about 3,000 elements, class-heavy, with a generated
//!   utility-first stylesheet of several thousand rules: a `*` reset that sets ~50 custom
//!   properties, spacing/colour/layout utilities, `hover:` and responsive `md:`/`lg:`
//!   variants inside `@media`, descendant component rules, and inline `style` attributes on
//!   some elements. Deterministic, no dependencies. This is the shape of the page that took
//!   12 s to style before the custom-property sharing fix.
//! - `wikipedia+2.2m`: the wikipedia fixture DOM with the 2.2 MB real-world sheet from
//!   `tests/data/css3-data`. About 18,000 rules, almost none of which match: the case that
//!   exercises the selector index and `@media` evaluation rather than the cascade itself.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use cow_utils::CowUtils as _;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use gosub_css3::matcher::styling::CssProperties;
use gosub_css3::stylesheet::{set_layout_viewport, CssStylesheet};
use gosub_css3::system::Css3System;
use gosub_css3::Css3;
use gosub_html5::document::document_impl::DocumentImpl;
use gosub_html5::html_compile;
use gosub_html5::parser::Html5Parser;
use gosub_interface::config::ModuleConfiguration;
use gosub_interface::css3::{CssOrigin, CssPropertyMap as _, CssSystem as _};
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_render_pipeline::common::document::pipeline_doc::GosubDocumentAdapter;
use gosub_render_pipeline::rendertree_builder::RenderTree;
use gosub_shared::config::ParserConfig;
use gosub_shared::node::NodeId;

#[derive(Clone, Debug, PartialEq)]
struct Config;

impl ModuleConfiguration for Config {
    type CssSystem = Css3System;
    type Document = DocumentImpl<Self>;
    type HtmlParser = Html5Parser<'static, Self>;
}

const VIEWPORT: (f32, f32) = (1280.0, 800.0);

// ── Fixtures ─────────────────────────────────────────────────────────────────

struct Fixture {
    name: &'static str,
    doc: Arc<DocumentImpl<Config>>,
    /// Elements that receive a computed style in one top-down pass.
    elements: u64,
}

fn parse_author_sheet(css: &str, name: &str) -> CssStylesheet {
    let config = ParserConfig {
        ignore_errors: true,
        ..Default::default()
    };
    Css3::parse_str(css, config, CssOrigin::Author, name).expect("bench stylesheet should parse")
}

fn build_fixture(name: &'static str, html: &str, author_css: &[(&str, &str)]) -> Fixture {
    let mut doc = html_compile::<Config>(html);
    doc.add_stylesheet(Css3System::load_default_useragent_stylesheet());
    for (sheet_name, css) in author_css {
        doc.add_stylesheet(parse_author_sheet(css, sheet_name));
    }
    let rules: usize = doc.stylesheets().iter().map(|s| s.rules.len()).sum();

    set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
    let elements = cascade_document(&doc);
    eprintln!(
        "fixture {name}: {} nodes, {elements} styled elements, {rules} rules",
        doc.node_count()
    );

    Fixture {
        name,
        doc: Arc::new(doc),
        elements,
    }
}

fn load_fixtures() -> Vec<Fixture> {
    let data_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/data");
    let wikipedia = std::fs::read_to_string(format!("{data_dir}/tree_iterator/wikipedia_main.html"))
        .expect("tests/data/tree_iterator/wikipedia_main.html should exist");
    let real_world_css = std::fs::read_to_string(format!("{data_dir}/css3-data/data.css"))
        .expect("tests/data/css3-data/data.css should exist");

    let utility = generate_utility_page();

    vec![
        build_fixture("small", SMALL_HTML, &[]),
        build_fixture("utility-3k", &utility.html, &[("utility.css", &utility.css)]),
        build_fixture("wikipedia+2.2m", &wikipedia, &[("data.css", &real_world_css)]),
    ]
}

// ── The two measurements ──────────────────────────────────────────────────────

/// Style every element top-down through the CSS crate alone. Returns how many got a map.
fn cascade_document(doc: &DocumentImpl<Config>) -> u64 {
    let mut styled = 0;
    let root = doc.root();
    style_subtree(doc, root, doc.stylesheets(), None, &mut styled);
    styled
}

fn style_subtree(
    doc: &DocumentImpl<Config>,
    id: NodeId,
    sheets: &[CssStylesheet],
    parent: Option<&CssProperties>,
    styled: &mut u64,
) {
    let own = if doc.node_type(id) == NodeType::ElementNode {
        let mut map = Css3System::properties_from_node::<Config>(doc, id, sheets, parent);
        if let Some(map) = map.as_mut() {
            // The adapter computes every property right after the cascade; so does this.
            for (_, property) in map.iter_mut() {
                black_box(property.compute_value());
            }
            *styled += 1;
        }
        map
    } else {
        None
    };

    // Text nodes take no style of their own; their children (none) and the element's children
    // inherit from the nearest element, which is what the adapter's flat-parent lookup does.
    let inherited = own.as_ref().or(parent);
    for &child in doc.children(id) {
        style_subtree(doc, child, sheets, inherited, styled);
    }
}

/// Stage 1 of the pipeline: a cold adapter and a render tree built over it.
fn build_render_tree(doc: &Arc<DocumentImpl<Config>>) -> usize {
    let adapter = GosubDocumentAdapter::<Config>::new(Arc::clone(doc));
    let mut render_tree = RenderTree::new(Arc::new(adapter));
    render_tree.parse().expect("render tree should build");
    render_tree.count_elements()
}

fn bench_style(c: &mut Criterion) {
    let fixtures = load_fixtures();

    let mut group = c.benchmark_group("style/cascade");
    group.sample_size(10).measurement_time(Duration::from_secs(15));
    for fixture in &fixtures {
        group.throughput(Throughput::Elements(fixture.elements));
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
            b.iter(|| black_box(cascade_document(&fixture.doc)));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("style/render-tree");
    group.sample_size(10).measurement_time(Duration::from_secs(15));
    for fixture in &fixtures {
        group.throughput(Throughput::Elements(fixture.elements));
        group.bench_with_input(BenchmarkId::from_parameter(fixture.name), fixture, |b, fixture| {
            set_layout_viewport(VIEWPORT.0, VIEWPORT.1);
            b.iter(|| black_box(build_render_tree(&fixture.doc)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_style);
criterion_main!(benches);

// ── Small fixture ─────────────────────────────────────────────────────────────

const SMALL_HTML: &str = r##"<!DOCTYPE html>
<html>
<head>
<style>
  body  { background: #eee; margin: 0; padding: 16px; }
  h1    { background: #333; color: #fff; padding: 8px; }
  .card { background: #fff; display: inline-block; width: 150px; height: 80px; margin: 8px; }
  .red  { background: #e74c3c; }
  .blue { background: #3498db; }
</style>
</head>
<body>
  <h1>Benchmark page</h1>
  <p>Lorem ipsum dolor sit amet, <a href="#">a link</a>.</p>
  <div class="card">A</div><div class="card red">B</div><div class="card blue">C</div>
  <div class="card">D</div><div class="card red">E</div><div class="card blue">F</div>
</body>
</html>"##;

// ── Generated utility-first page ──────────────────────────────────────────────

struct GeneratedPage {
    html: String,
    css: String,
}

/// A small deterministic generator, so the page is the same on every run and every machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

const HUES: [&str; 10] = [
    "gray", "red", "orange", "amber", "green", "teal", "blue", "indigo", "purple", "pink",
];
const SHADES: [u32; 10] = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900];
const SPACING: [(&str, &str); 24] = [
    ("0", "0px"),
    ("px", "1px"),
    ("0.5", "0.125rem"),
    ("1", "0.25rem"),
    ("1.5", "0.375rem"),
    ("2", "0.5rem"),
    ("2.5", "0.625rem"),
    ("3", "0.75rem"),
    ("3.5", "0.875rem"),
    ("4", "1rem"),
    ("5", "1.25rem"),
    ("6", "1.5rem"),
    ("7", "1.75rem"),
    ("8", "2rem"),
    ("9", "2.25rem"),
    ("10", "2.5rem"),
    ("11", "2.75rem"),
    ("12", "3rem"),
    ("14", "3.5rem"),
    ("16", "4rem"),
    ("20", "5rem"),
    ("24", "6rem"),
    ("32", "8rem"),
    ("40", "10rem"),
];
const TEXT_SIZES: [(&str, &str, &str); 8] = [
    ("xs", "0.75rem", "1rem"),
    ("sm", "0.875rem", "1.25rem"),
    ("base", "1rem", "1.5rem"),
    ("lg", "1.125rem", "1.75rem"),
    ("xl", "1.25rem", "1.75rem"),
    ("2xl", "1.5rem", "2rem"),
    ("3xl", "1.875rem", "2.25rem"),
    ("4xl", "2.25rem", "2.5rem"),
];
const BREAKPOINTS: [(&str, u32); 3] = [("sm", 640), ("md", 768), ("lg", 1024)];

fn generate_utility_css() -> String {
    let mut css = String::with_capacity(600_000);

    // The reset: one rule on every element, carrying the custom properties the shadow, ring
    // and transform utilities read through var(). This is the pattern that made styling
    // allocation-bound before the custom-property map was shared with the parent.
    css.push_str(
        "*, ::before, ::after { box-sizing: border-box; border-width: 0; border-style: solid; border-color: #e5e7eb;",
    );
    for i in 0..50 {
        let _ = write!(css, " --tw-v{i}: 0;");
    }
    css.push_str(" --tw-shadow: 0 0 #0000; --tw-ring-shadow: 0 0 #0000; --tw-ring-offset-shadow: 0 0 #0000; }\n");

    // Preflight-style base rules.
    css.push_str(concat!(
        "html { line-height: 1.5; -webkit-text-size-adjust: 100%; font-family: ui-sans-serif, system-ui, sans-serif; }\n",
        "body { margin: 0; line-height: inherit; }\n",
        "h1, h2, h3, h4, h5, h6 { font-size: inherit; font-weight: inherit; margin: 0; }\n",
        "a { color: inherit; text-decoration: inherit; }\n",
        "p, ul, ol, figure { margin: 0; }\n",
        "ul, ol { list-style: none; padding: 0; }\n",
        "img, svg, video { display: block; vertical-align: middle; max-width: 100%; height: auto; }\n",
        "button, input, select, textarea { font-family: inherit; font-size: 100%; margin: 0; padding: 0; color: inherit; }\n",
        "button { background-color: transparent; background-image: none; cursor: pointer; }\n",
    ));

    // Utilities that also get responsive copies.
    let mut responsive = String::new();
    let _ = writeln!(responsive, ".block {{ display: block; }} .inline-block {{ display: inline-block; }} .inline {{ display: inline; }} .flex {{ display: flex; }} .inline-flex {{ display: inline-flex; }} .grid {{ display: grid; }} .hidden {{ display: none; }}");
    let _ = writeln!(responsive, ".flex-row {{ flex-direction: row; }} .flex-col {{ flex-direction: column; }} .flex-wrap {{ flex-wrap: wrap; }} .flex-1 {{ flex: 1 1 0%; }} .items-center {{ align-items: center; }} .items-start {{ align-items: flex-start; }} .justify-between {{ justify-content: space-between; }} .justify-center {{ justify-content: center; }}");
    for cols in 1..=12 {
        let _ = writeln!(
            responsive,
            ".grid-cols-{cols} {{ grid-template-columns: repeat({cols}, minmax(0, 1fr)); }}"
        );
        let _ = writeln!(
            responsive,
            ".col-span-{cols} {{ grid-column: span {cols} / span {cols}; }}"
        );
    }
    for (key, value) in SPACING {
        // `.p-0.5` would tokenize as an ident followed by the number `.5`; a class name with a
        // dot in it is written `.p-0\.5`, which is what utility frameworks emit.
        let key = key.cow_replace('.', "\\.");
        for (prefix, property) in [
            ("p", "padding"),
            ("m", "margin"),
            ("gap", "gap"),
            ("w", "width"),
            ("h", "height"),
        ] {
            let _ = writeln!(responsive, ".{prefix}-{key} {{ {property}: {value}; }}");
        }
        for (prefix, a, b) in [
            ("px", "padding-left", "padding-right"),
            ("py", "padding-top", "padding-bottom"),
            ("mx", "margin-left", "margin-right"),
            ("my", "margin-top", "margin-bottom"),
        ] {
            let _ = writeln!(responsive, ".{prefix}-{key} {{ {a}: {value}; {b}: {value}; }}");
        }
        for (prefix, property) in [
            ("pt", "padding-top"),
            ("pb", "padding-bottom"),
            ("mt", "margin-top"),
            ("mb", "margin-bottom"),
        ] {
            let _ = writeln!(responsive, ".{prefix}-{key} {{ {property}: {value}; }}");
        }
    }
    for (name, size, line_height) in TEXT_SIZES {
        let _ = writeln!(
            responsive,
            ".text-{name} {{ font-size: {size}; line-height: {line_height}; }}"
        );
    }
    let _ = writeln!(responsive, ".w-full {{ width: 100%; }} .h-full {{ height: 100%; }} .max-w-7xl {{ max-width: 80rem; }} .mx-auto {{ margin-left: auto; margin-right: auto; }} .w-1\\/2 {{ width: 50%; }} .w-1\\/3 {{ width: 33.333333%; }} .w-2\\/3 {{ width: 66.666667%; }}");
    css.push_str(&responsive);

    // Colours: text, background and border for every hue and shade.
    for (h, hue) in HUES.iter().enumerate() {
        for (s, shade) in SHADES.iter().enumerate() {
            let channel = |offset: usize| 40 + ((h * 37 + s * 19 + offset * 53) % 200);
            let (r, g, b) = (channel(0), channel(1), channel(2));
            let _ = writeln!(
                css,
                ".text-{hue}-{shade} {{ --tw-text-opacity: 1; color: rgb({r} {g} {b} / var(--tw-text-opacity)); }}"
            );
            let _ = writeln!(css, ".bg-{hue}-{shade} {{ --tw-bg-opacity: 1; background-color: rgb({r} {g} {b} / var(--tw-bg-opacity)); }}");
            let _ = writeln!(css, ".border-{hue}-{shade} {{ border-color: rgb({r} {g} {b}); }}");
            let _ = writeln!(
                css,
                ".hover\\:bg-{hue}-{shade}:hover {{ background-color: rgb({r} {g} {b}); }}"
            );
            let _ = writeln!(css, ".hover\\:text-{hue}-{shade}:hover {{ color: rgb({r} {g} {b}); }}");
        }
    }
    css.push_str(".text-white { color: #fff; } .bg-white { background-color: #fff; } .bg-transparent { background-color: transparent; }\n");

    // Typography, borders, effects. The shadow and ring utilities read the reset's variables.
    css.push_str(concat!(
        ".font-normal { font-weight: 400; } .font-medium { font-weight: 500; } .font-semibold { font-weight: 600; } .font-bold { font-weight: 700; }\n",
        ".italic { font-style: italic; } .uppercase { text-transform: uppercase; } .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }\n",
        ".leading-tight { line-height: 1.25; } .leading-relaxed { line-height: 1.625; } .tracking-wide { letter-spacing: 0.025em; }\n",
        ".text-left { text-align: left; } .text-center { text-align: center; } .underline { text-decoration-line: underline; } .hover\\:underline:hover { text-decoration-line: underline; }\n",
        ".border { border-width: 1px; } .border-2 { border-width: 2px; } .border-b { border-bottom-width: 1px; } .border-t { border-top-width: 1px; }\n",
        ".rounded { border-radius: 0.25rem; } .rounded-md { border-radius: 0.375rem; } .rounded-lg { border-radius: 0.5rem; } .rounded-full { border-radius: 9999px; }\n",
        ".shadow-sm { --tw-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05); box-shadow: var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow); }\n",
        ".shadow { --tw-shadow: 0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1); box-shadow: var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow); }\n",
        ".shadow-lg { --tw-shadow: 0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1); box-shadow: var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow); }\n",
        ".ring-1 { --tw-ring-shadow: 0 0 0 1px rgb(59 130 246 / 0.5); box-shadow: var(--tw-ring-offset-shadow), var(--tw-ring-shadow), var(--tw-shadow); }\n",
        ".opacity-75 { opacity: 0.75; } .opacity-50 { opacity: 0.5; } .overflow-hidden { overflow: hidden; } .relative { position: relative; } .absolute { position: absolute; } .sticky { position: sticky; top: 0; }\n",
        ".inset-0 { top: 0; right: 0; bottom: 0; left: 0; } .z-10 { z-index: 10; } .cursor-pointer { cursor: pointer; } .select-none { user-select: none; }\n",
        ".transition { transition-property: color, background-color, border-color, opacity, box-shadow, transform; transition-duration: 150ms; }\n",
        ".object-cover { object-fit: cover; } .aspect-video { aspect-ratio: 16 / 9; } .min-h-screen { min-height: 100vh; } .space-y-2 > * + * { margin-top: 0.5rem; } .space-x-2 > * + * { margin-left: 0.5rem; }\n",
    ));

    // Responsive variants: the layout utilities again, once per breakpoint, inside @media.
    for (prefix, min_width) in BREAKPOINTS {
        let _ = writeln!(css, "@media (min-width: {min_width}px) {{");
        for line in responsive.lines() {
            // `.p-4 { ... }` becomes `.md\:p-4 { ... }` for every rule on the line.
            let variant = line.cow_replace(" .", &format!(" .{prefix}\\:"));
            let variant = if let Some(rest) = variant.strip_prefix('.') {
                format!(".{prefix}\\:{rest}")
            } else {
                variant.into_owned()
            };
            css.push_str("  ");
            css.push_str(&variant);
            css.push('\n');
        }
        css.push_str("}\n");
    }

    // Component rules with descendant and pseudo-class selectors, the way hand-written CSS
    // on top of a utility framework tends to look.
    css.push_str(concat!(
        ".site-header { background: linear-gradient(90deg, #1e3a8a, #3b82f6); color: white; }\n",
        ".site-header nav a { padding: 0.5rem 0.75rem; border-radius: 0.375rem; }\n",
        ".site-header nav a:hover { background-color: rgb(255 255 255 / 0.1); }\n",
        ".section + .section { border-top: 1px solid #e5e7eb; }\n",
        ".section > h2 { margin-bottom: 1rem; }\n",
        ".card { transition: box-shadow 150ms ease; }\n",
        ".card:hover { box-shadow: 0 10px 15px -3px rgb(0 0 0 / 0.1); }\n",
        ".card > figure img { width: 100%; }\n",
        ".card .meta span:not(:last-child)::after { content: \"\\00b7\"; margin: 0 0.375rem; color: #9ca3af; }\n",
        ".card .tags li { font-size: 0.75rem; line-height: 1rem; padding: 0.125rem 0.5rem; border-radius: 9999px; background: #f3f4f6; }\n",
        ".card .tags li:nth-child(odd) { background: #e5e7eb; }\n",
        ".card:nth-child(3n) .badge { background-color: #fde68a; }\n",
        ".section .card p a { color: #2563eb; }\n",
        ".section .card p a:hover { text-decoration: underline; }\n",
        "article.card h3 { font-size: 1.125rem; line-height: 1.75rem; font-weight: 600; }\n",
        ".btn { display: inline-flex; align-items: center; padding: 0.5rem 1rem; border-radius: 0.375rem; font-weight: 500; }\n",
        ".btn-primary { background-color: #2563eb; color: white; }\n",
        ".btn-primary:hover { background-color: #1d4ed8; }\n",
        ".btn-ghost { border: 1px solid #d1d5db; }\n",
        "footer.site-footer a { color: #6b7280; }\n",
        "footer.site-footer a:hover { color: #111827; }\n",
        "@media (max-width: 639px) { .section > h2 { font-size: 1.25rem; } .card { margin-bottom: 1rem; } }\n",
        "@media (prefers-color-scheme: dark) { body { background-color: #111827; color: #f9fafb; } .card { background-color: #1f2937; } }\n",
    ));

    css
}

fn generate_utility_html() -> String {
    let mut rng = Lcg(0x5eed_5eed_5eed);
    let mut html = String::with_capacity(400_000);

    let text_colors: Vec<String> = HUES
        .iter()
        .flat_map(|hue| SHADES.iter().map(move |shade| format!("text-{hue}-{shade}")))
        .collect();
    let bg_colors: Vec<String> = HUES
        .iter()
        .flat_map(|hue| SHADES.iter().map(move |shade| format!("bg-{hue}-{shade}")))
        .collect();
    let text_color_refs: Vec<&str> = text_colors.iter().map(String::as_str).collect();
    let bg_color_refs: Vec<&str> = bg_colors.iter().map(String::as_str).collect();

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Utility benchmark page</title>\n</head>\n<body class=\"min-h-screen bg-gray-50 text-gray-900\">\n");

    html.push_str("<header class=\"site-header sticky z-10 shadow\">\n<div class=\"max-w-7xl mx-auto px-4 flex items-center justify-between h-16\">\n");
    html.push_str(
        "<a href=\"/\" class=\"text-xl font-bold tracking-wide\">Bench</a>\n<nav class=\"hidden md:flex gap-2\">\n",
    );
    for item in ["Home", "Articles", "Topics", "About", "Contact"] {
        let _ = writeln!(
            html,
            "<a href=\"#\" class=\"text-sm font-medium hover:underline\">{item}</a>"
        );
    }
    html.push_str("</nav>\n<button class=\"btn btn-ghost md:hidden\" style=\"padding: 6px 10px\">Menu</button>\n</div>\n</header>\n");

    html.push_str("<main class=\"max-w-7xl mx-auto px-4 py-8 space-y-2\">\n");

    for section in 0..12 {
        let cols = rng.pick(&[
            "md:grid-cols-2 lg:grid-cols-3",
            "md:grid-cols-2 lg:grid-cols-4",
            "md:grid-cols-3",
        ]);
        let _ = writeln!(
            html,
            "<section id=\"section-{section}\" class=\"section py-8\">\n<h2 class=\"text-2xl font-bold {}\">Section {section}</h2>\n<div class=\"grid grid-cols-1 gap-6 {cols}\">",
            rng.pick(&text_color_refs)
        );

        for card in 0..8 {
            let card_bg = rng.pick(&["bg-white", "bg-gray-50", "bg-gray-100"]);
            let radius = rng.pick(&["rounded", "rounded-md", "rounded-lg"]);
            let shadow = rng.pick(&["shadow-sm", "shadow", "shadow-lg"]);
            let inline = if rng.below(10) == 0 {
                format!(
                    " style=\"margin-top: {}px; opacity: 0.9{}\"",
                    rng.below(12),
                    rng.below(10)
                )
            } else {
                String::new()
            };
            let _ = writeln!(
                html,
                "<article class=\"card {card_bg} {radius} {shadow} overflow-hidden flex flex-col transition\"{inline}>"
            );
            let _ = writeln!(
                html,
                "<figure class=\"aspect-video {} relative\"><img src=\"img-{section}-{card}.jpg\" alt=\"\" class=\"object-cover w-full h-full\"><span class=\"badge absolute z-10 text-xs font-semibold px-2 py-0.5 rounded-full {} text-white\" style=\"top: 8px; left: 8px\">Tag {}</span></figure>",
                rng.pick(&bg_color_refs),
                rng.pick(&bg_color_refs),
                rng.below(20)
            );
            html.push_str("<div class=\"p-4 flex flex-col gap-2 flex-1\">\n");
            let _ = writeln!(
                html,
                "<h3 class=\"text-lg font-semibold leading-tight truncate\"><a href=\"#\" class=\"{} hover:underline\">Card {section}.{card}: a heading that runs a little long</a></h3>",
                rng.pick(&text_color_refs)
            );
            html.push_str("<div class=\"meta flex flex-wrap items-center text-xs text-gray-500\">");
            let _ = write!(
                html,
                "<span>{} min read</span><span>{} views</span><span class=\"uppercase tracking-wide\">{}</span>",
                2 + rng.below(20),
                100 + rng.below(9_000),
                rng.pick(&["news", "essay", "review", "guide"])
            );
            html.push_str("</div>\n");
            let _ = writeln!(
                html,
                "<p class=\"text-sm text-gray-600 leading-relaxed\">Lorem ipsum dolor sit amet, <a href=\"#\" class=\"font-medium\">consectetur</a> adipiscing elit, sed do <em class=\"italic\">eiusmod</em> tempor <strong class=\"font-semibold\">incididunt</strong> ut labore et dolore magna aliqua.</p>"
            );
            html.push_str("<ul class=\"tags flex flex-wrap gap-2 mt-2\">");
            for tag in 0..(2 + rng.below(4)) {
                let _ = write!(
                    html,
                    "<li class=\"tag\">tag-{}</li>",
                    (section * 7 + card * 3 + tag) % 23
                );
            }
            html.push_str("</ul>\n");
            let _ = writeln!(
                html,
                "<div class=\"mt-2 flex items-center justify-between\"><a href=\"#\" class=\"btn btn-primary text-sm\">Read more</a><button class=\"btn btn-ghost text-sm {}\">Save</button></div>",
                rng.pick(&["", "opacity-75", "hover:bg-gray-100"])
            );
            html.push_str("</div>\n</article>\n");
        }
        html.push_str("</div>\n</section>\n");
    }

    html.push_str("</main>\n<footer class=\"site-footer border-t py-8 mt-8\">\n<div class=\"max-w-7xl mx-auto px-4 grid grid-cols-2 md:grid-cols-4 gap-6 text-sm\">\n");
    for column in 0..4 {
        let _ = writeln!(
            html,
            "<div><h4 class=\"font-semibold mb-2\">Column {column}</h4><ul class=\"space-y-2\">"
        );
        for link in 0..6 {
            let _ = writeln!(
                html,
                "<li><a href=\"#\" class=\"hover:underline\">Link {column}.{link}</a></li>"
            );
        }
        html.push_str("</ul></div>\n");
    }
    html.push_str("</div>\n</footer>\n</body>\n</html>\n");

    html
}

fn generate_utility_page() -> GeneratedPage {
    GeneratedPage {
        html: generate_utility_html(),
        css: generate_utility_css(),
    }
}
