//! Drives the network process end to end, from a binary that dispatches child
//! roles the way a real embedder does.

use gosub_interface::font_system::{Confinement, FontSystem};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const BODY: &str = "<html><head><title>through the net process</title></head>\
<body style=\"margin:0\"><a href=\"https://example.test/target\" \
style=\"display:block;width:400px;height:200px\">a link to hover</a></body></html>";

/// The harness's render configuration: null backend and compositor (nothing
/// composites here), the scenario-selected font system - and, behind the
/// `cairo-tiles` feature, the Cairo CPU rasterizer for forked renderers.
struct TileConfig<F>(std::marker::PhantomData<F>);

impl<F> Clone for TileConfig<F> {
    fn clone(&self) -> Self {
        Self(std::marker::PhantomData)
    }
}
impl<F> std::fmt::Debug for TileConfig<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TileConfig")
    }
}
impl<F> PartialEq for TileConfig<F> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<F: FontSystem + Default> gosub_interface::config::ModuleConfiguration for TileConfig<F> {
    type CssSystem = gosub_css3::system::Css3System;
    type Document = gosub_html5::document::document_impl::DocumentImpl<Self>;
    type HtmlParser = gosub_html5::parser::Html5Parser<'static, Self>;
}

impl<F: FontSystem + Default> gosub_engine::html::RenderConfiguration for TileConfig<F> {
    type RenderBackend = gosub_render_pipeline::render::backends::null::NullBackend;
    type CompositorSink = gosub_render_pipeline::render::DefaultCompositor;
    type FontSystem = F;

    fn forked_tile_rasterizer(
        font_system: std::sync::Arc<parking_lot::Mutex<dyn FontSystem>>,
    ) -> Option<Box<dyn gosub_render_pipeline::rasterizer::Rasterable + Send + Sync>> {
        #[cfg(feature = "cairo-tiles")]
        {
            Some(Box::new(gosub_renderer_cairo::CairoRasterizer::with_font_system(
                font_system,
            )))
        }
        #[cfg(not(feature = "cairo-tiles"))]
        {
            let _ = font_system;
            None
        }
    }
}

/// Run a font scenario against the font system named by argv[2].
macro_rules! with_font_backend {
    ($scenario:ident) => {{
        let backend = std::env::args().nth(2).unwrap_or_else(|| "parley".into());
        // Spawned children (the fork server) inherit this, which is how a
        // re-exec - where type parameters cannot travel - ends up dispatching
        // with the same font system the scenario is testing. A production
        // embedder has no such indirection: it names its one font system in
        // `dispatch_with` directly.
        std::env::set_var("GOSUB_HARNESS_FONT_BACKEND", &backend);
        match backend.as_str() {
            "parley" => $scenario::<gosub_fontmanager::ParleyFontSystem>(),
            "cosmic" => $scenario::<gosub_fontmanager::CosmicFontSystem>(),
            #[cfg(feature = "pango-fonts")]
            "pango" => $scenario::<gosub_fontmanager::PangoFontSystem>(),
            #[cfg(feature = "skia-fonts")]
            "skia" => $scenario::<gosub_fontmanager::SkiaFontSystem>(),
            other => {
                eprintln!(
                    "font backend {other:?} is not compiled into this harness; \
                     'parley' and 'cosmic' are always available, 'pango' and 'skia' \
                     need the engine features 'pango-fonts' / 'skia-fonts'"
                );
                2
            }
        }
    }};
}

fn main() {
    // First statement, exactly as the docs require of an embedder: in a child
    // this runs the role and exits, so nothing below executes there. Skipping it
    // is the mistake the `guard` scenario reproduces.
    if std::env::var_os("GOSUB_HARNESS_SKIP_DISPATCH").is_none() {
        match std::env::var("GOSUB_HARNESS_FONT_BACKEND").as_deref() {
            Ok("cosmic") => {
                gosub_engine::child_process::dispatch_with::<TileConfig<gosub_fontmanager::CosmicFontSystem>>()
            }
            #[cfg(feature = "pango-fonts")]
            Ok("pango") => {
                gosub_engine::child_process::dispatch_with::<TileConfig<gosub_fontmanager::PangoFontSystem>>()
            }
            #[cfg(feature = "skia-fonts")]
            Ok("skia") => gosub_engine::child_process::dispatch_with::<TileConfig<gosub_fontmanager::SkiaFontSystem>>(),
            _ => gosub_engine::child_process::dispatch_with::<TileConfig<gosub_fontmanager::ParleyFontSystem>>(),
        }
    }

    let scenario = std::env::args().nth(1).unwrap_or_default();
    let code = match scenario.as_str() {
        "direct" => direct(),
        "engine" => engine(),
        "guard" => guard(),
        "decode" => decode(),
        "decode-garbage" => decode_garbage(),
        "decode-many" => decode_many(),
        "fonts-under-lockdown" => with_font_backend!(fonts_under_lockdown),
        "webfont-under-lockdown" => with_font_backend!(webfont_under_lockdown),
        "fonts-under-font-readable-lockdown" => with_font_backend!(fonts_under_font_readable_lockdown),
        "webfont-under-font-readable-lockdown" => with_font_backend!(webfont_under_font_readable_lockdown),
        "fork-server" => with_font_backend!(fork_server_roundtrip),
        "render-under-lockdown" => with_font_backend!(render_under_lockdown),
        "engine-renderer-process" => with_font_backend!(engine_renderer_process),
        "exec-renderer" => with_font_backend!(exec_renderer_roundtrip),
        "renderer-lifecycle" => with_font_backend!(renderer_lifecycle),
        "renderer-scroll-window" => with_font_backend!(renderer_scroll_window),
        "renderer-hover" => with_font_backend!(renderer_hover),
        "render-file" => with_font_backend!(render_file),
        "render-file-locked" => with_font_backend!(render_file_locked),
        other => {
            eprintln!("unknown scenario {other:?}; expected 'direct' or 'engine'");
            2
        }
    };
    std::process::exit(code);
}

/// A 2x2 RGBA PNG: red, green, blue, white.
const SAMPLE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xB6, 0x0D, 0x24, 0x00, 0x00, 0x00, 0x12, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x0C, 0x81, 0x34, 0x18, 0x00, 0x00, 0x49, 0xC8,
    0x09, 0xF7, 0xF9, 0xAB, 0xB6, 0x0D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The decode boundary: real image bytes go into a throwaway process and the
/// exact pixels come back.
fn decode() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::{BrokeredDecode, ImageDecoder};

    match ProcessImageDecoder.decode(Some("image/png"), SAMPLE_PNG) {
        Ok(BrokeredDecode::Raster(image)) => {
            if (image.width, image.height) != (2, 2) {
                eprintln!("expected a 2x2 image, got {}x{}", image.width, image.height);
                return 1;
            }
            let expected: &[u8] = &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255];
            if image.rgba.as_ref() != expected {
                eprintln!("pixels did not survive the boundary: {:?}", image.rgba.as_ref());
                return 1;
            }
        }
        Ok(BrokeredDecode::Vector) => {
            eprintln!("a PNG should not decode as a vector");
            return 1;
        }
        Err(e) => {
            eprintln!("decode in a separate process failed: {e}");
            return 1;
        }
    }

    // SVG is the format whose decoder discovers system fonts on first use -
    // a filesystem walk the decoder sandbox forbids, so it has to happen
    // before the lockdown. A logo-sized SVG (with text, to make the fontdb
    // matter) must come back as a vector, not as a dead decoder.
    match ProcessImageDecoder.decode(Some("image/svg+xml"), SAMPLE_SVG) {
        Ok(BrokeredDecode::Vector) => 0,
        Ok(BrokeredDecode::Raster(_)) => {
            eprintln!("an SVG should decode as a vector, not a raster");
            1
        }
        Err(e) => {
            eprintln!("SVG decode in a separate process failed: {e}");
            1
        }
    }
}

/// A small SVG with a `<text>` element, so decoding it consults the fontdb.
const SAMPLE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18"><rect width="18" height="18" fill="#f60"/><text x="4" y="13" font-family="serif" font-size="10">Y</text></svg>"##;

/// The open question for a renderer process: can text be laid out by a process
/// confined the way a renderer must be?
fn fonts_under_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::TextStyle;

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());

    // Warm-up. `families()` is documented to populate lazily-built databases, and
    // resolving plus shaping forces the actual file reads that follow.
    let families = fonts.families();
    println!("warm-up: {} families visible before lockdown", families.len());
    if families.is_empty() {
        eprintln!("no font families found; this host cannot answer the question");
        return 2;
    }
    // Exercise the trait hook rather than hand-rolled warm-up: this is what a
    // renderer will call, and it is the configured font system's own answer to
    // "get ready to be confined". Timed and measured, because the cost of that
    // answer is part of whether the strategy is usable. This scenario tests the
    // *full* lockdown, so any answer below `Full` ends it here - the tiered
    // scenarios cover the rest.
    let rss_before_mib = rss_mib();
    let start = std::time::Instant::now();
    match fonts.prepare_for_confinement() {
        Confinement::Full => {}
        other => {
            eprintln!("this font system does not support full confinement: {other:?}");
            return 3;
        }
    }
    println!(
        "prepare_for_confinement over {} families: {:?}, RSS {} -> {} MiB",
        families.len(),
        start.elapsed(),
        rss_before_mib,
        rss_mib()
    );

    gosub_sandbox::lock_down_renderer();

    // Text and a size never used above, so any per-face lazy load still pending
    // has to happen now - after the sandbox is in place.
    let cold_style = TextStyle::new("serif", 31.0);
    let (cold_w, cold_h) = fonts.measure("Text shaped only after the sandbox applied", &cold_style);
    if cold_w <= 0.0 || cold_h <= 0.0 {
        eprintln!("shaping under lockdown produced an empty box ({cold_w}x{cold_h})");
        return 1;
    }
    println!("shaped {cold_w:.1}x{cold_h:.1} under the renderer lockdown");

    // A real page runs hundreds of layouts before it first needs some face.
    // Parley prunes its source cache every layout (entries idle for 128
    // layouts go), so a warm-up that merely *loaded* every face is undone by
    // the time a long page reaches a face it has not used yet - and the
    // reload is a file read. Churn past that window, then ask for a face
    // nothing above has touched.
    let churn_style = TextStyle::new("sans-serif", 13.0);
    for i in 0..300 {
        let _ = fonts.measure(&format!("layout {i}"), &churn_style);
    }
    let mut late_style = TextStyle::new("serif", 27.0);
    late_style.weight = gosub_interface::font_system::FontWeight(700);
    late_style.style = gosub_interface::font::FontStyle::Italic;
    let (late_w, late_h) = fonts.measure("Bold italic serif, first used after 300 layouts", &late_style);
    if late_w <= 0.0 || late_h <= 0.0 {
        eprintln!("shaping a late face under lockdown produced an empty box ({late_w}x{late_h})");
        return 1;
    }
    println!("shaped a never-before-used face after 300 layouts under lockdown ({late_w:.1}x{late_h:.1})");
    0
}

/// The middle tier: can a font system that *cannot* be confined outright
/// (fontconfig consults the filesystem while shaping) run in a renderer that is
/// allowed to read font paths and nothing else?
fn fonts_under_font_readable_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::TextStyle;

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());

    let families = fonts.families();
    println!("{} families visible before lockdown", families.len());
    if families.is_empty() {
        eprintln!("no font families found; this host cannot answer the question");
        return 2;
    }

    // A runtime guard rather than a cfg'd block, so the code below it is not
    // flagged unreachable on the platforms this returns on.
    if cfg!(not(target_os = "linux")) {
        eprintln!("the font-readable renderer tier exists only on Linux");
        return 2;
    }
    #[cfg(target_os = "linux")]
    {
        let paths = gosub_sandbox::font_filesystem_paths();
        if paths.is_empty() {
            eprintln!("no font paths exist on this host; the profile would test nothing");
            return 2;
        }
        println!(
            "font paths granted read-only: {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
        gosub_sandbox::lock_down_renderer_with_font_access(&refs);
    }

    // Never shaped, never warmed: the match runs cold, under the profile.
    let cold_style = TextStyle::new("serif", 31.0);
    let (cold_w, cold_h) = fonts.measure("Text shaped only after the sandbox applied", &cold_style);
    if cold_w <= 0.0 || cold_h <= 0.0 {
        eprintln!("shaping under the font-readable lockdown produced an empty box ({cold_w}x{cold_h})");
        return 1;
    }
    println!("shaped {cold_w:.1}x{cold_h:.1} under the font-readable renderer lockdown");
    0
}

/// Web fonts under the middle tier - the follow-up that decides whether the
/// tier is actually usable, because `@font-face` is everywhere.
fn webfont_under_font_readable_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::{FontQuery, TextStyle};

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());
    let _ = fonts.families();

    let Ok(resolved) = fonts.resolve(&FontQuery::new(&["sans-serif"])) else {
        eprintln!("no resolvable font on this host to use as sample bytes");
        return 2;
    };
    let downloaded: Vec<u8> = resolved.blob.data.as_ref().as_ref().to_vec();
    if downloaded.is_empty() {
        eprintln!("resolved font carried no bytes; the control is broken");
        return 2;
    }
    println!("holding {} bytes of font data before lockdown", downloaded.len());

    if cfg!(not(target_os = "linux")) {
        eprintln!("the font-readable renderer tier exists only on Linux");
        return 2;
    }
    #[cfg(target_os = "linux")]
    {
        let scratch = std::env::temp_dir().join(format!("gosub-webfont-scratch-{}", std::process::id()));
        if std::fs::create_dir_all(&scratch).is_err() {
            eprintln!("could not create the scratch directory; cannot set the tier up");
            return 2;
        }
        // Backends that stage fonts as files use the standard temp dir; point
        // it inside the ruleset so the write is scoped, not just allowed.
        std::env::set_var("TMPDIR", &scratch);

        let paths = gosub_sandbox::font_filesystem_paths();
        let mut refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
        refs.push((scratch.as_path(), true));
        gosub_sandbox::lock_down_renderer_with_font_access(&refs);
    }

    if let Err(e) = fonts.register_font(downloaded, Some("gosub-webfont-test")) {
        eprintln!("registering a web font under the font-readable lockdown failed: {e:?}");
        return 1;
    }
    let (w, h) = fonts.measure(
        "Web font registered after the sandbox applied",
        &TextStyle::new(resolved.family.clone(), 24.0),
    );
    if w <= 0.0 || h <= 0.0 {
        eprintln!("shaping with the registered font produced an empty box ({w}x{h})");
        return 1;
    }
    println!("registered and shaped {w:.1}x{h:.1} under the font-readable renderer lockdown");
    0
}

/// The render pipeline under the renderer lockdown, in *this* process - the
/// same stages the forked renderer runs, minus the fork machinery, so a
/// pipeline-vs-sandbox failure can be debugged (and bisected) directly.
fn render_under_lockdown<F: FontSystem + Default>() -> i32 {
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::renderer;

        let mut fonts = F::default();
        println!("font backend: {}", std::any::type_name::<F>());
        let _ = fonts.families();
        match fonts.prepare_for_confinement() {
            Confinement::Full => {}
            other => {
                eprintln!("this scenario needs a fully-confinable font system, got {other:?}");
                return 2;
            }
        }

        // SVG text goes through a system fontdb that loads face files lazily;
        // pin them before the lockdown, exactly as the fork server does.
        gosub_render_pipeline::common::media::SvgDecoder::pin_system_fonts();
        let media_store = std::sync::Arc::new(gosub_render_pipeline::common::media::MediaStore::new());

        gosub_sandbox::lock_down_renderer();

        let shared: std::sync::Arc<parking_lot::Mutex<dyn FontSystem>> =
            std::sync::Arc::new(parking_lot::Mutex::new(fonts));
        let (summary, baked, _hit_regions) = renderer::render_page::<TileConfig<F>>(
            renderer::PageRequest {
                html: "<html><body><h1>Under lockdown</h1><p>Rendered without a fork.</p></body></html>",
                page_url: "about:blank",
                viewport_width: 1280.0,
                viewport_height: 720.0,
                known_tiles: &Default::default(),
                hovered_node: None,
            },
            shared,
            media_store,
            std::sync::Arc::new(gosub_interface::resource_loader::NoResourceLoader),
        );
        if summary.page_height <= 0.0 || summary.paint_commands == 0 {
            eprintln!("implausible page under lockdown: {summary:?}");
            return 1;
        }
        // With a rasterizer compiled in, stage 6 must run under this sandbox
        // too, and produce pixels that are actually ink rather than zeroes.
        if cfg!(feature = "cairo-tiles") {
            let inked: u64 = baked
                .iter()
                .map(|tile| match tile {
                    renderer::RenderedTile::Fresh { tile, .. } => match &tile.pixels {
                        gosub_render_pipeline::common::texture::TilePixels::Cpu(bytes) => {
                            bytes.iter().filter(|&&b| b != 0).count() as u64
                        }
                        gosub_render_pipeline::common::texture::TilePixels::Gpu(_) => 0,
                    },
                    // Nothing was rasterized here (no broker memory in this
                    // scenario, so this cannot happen).
                    renderer::RenderedTile::Unchanged { .. } => 0,
                })
                .sum();
            if baked.is_empty() || inked == 0 {
                eprintln!("rasterization under lockdown produced no ink ({} tiles)", baked.len());
                return 1;
            }
            println!(
                "rasterized {} tiles under lockdown ({inked} non-zero bytes)",
                baked.len()
            );
        }
        println!(
            "rendered a {:.0}x{:.0} page under the renderer lockdown ({} paint commands)",
            summary.page_width, summary.page_height, summary.paint_commands
        );
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the renderer lockdown exists only on Linux");
        2
    }
}

/// The exec-fresh renderer, driven directly: spawn one throwaway
/// font-readable-confined renderer process, have it render the same
/// css+font+img page the fork-server scenario uses, and verify the brokered
/// loads and the streamed tiles - the `FontPathsReadable` tier's whole
/// render path, without an engine around it.
fn exec_renderer_roundtrip<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        let html = r#"
            <html><head>
            <link rel="stylesheet" href="/page.css">
            <style> body { margin: 0; } h1 { font-size: 32px; } </style></head>
            <body>
                <h1>Rendered in an exec'd renderer</h1>
                <div class="card"><p>One page, one process, gone.</p></div>
                <img src="/tile.png" width="64" height="64">
            </body></html>
        "#;
        let font_bytes = {
            use gosub_interface::font_system::FontQuery;
            let mut fonts = gosub_fontmanager::ParleyFontSystem::default();
            let _ = fonts.families();
            fonts
                .resolve(&FontQuery::new(&["sans-serif"]))
                .map(|resolved| resolved.blob.data.as_ref().as_ref().to_vec())
                .unwrap_or_default()
        };
        let loader = HarnessResourceLoader {
            font: font_bytes,
            ..Default::default()
        };

        match gosub_engine::render_process::client::render_page(
            html,
            "http://harness.invalid/index.html",
            "exec-harness-tab",
            (1280.0, 720.0),
            &loader,
            &Default::default(),
            None,
        ) {
            Ok(page) => {
                let (summary, tiles) = (page.summary, page.tiles);
                if summary.page_height < 300.0 || summary.paint_commands == 0 {
                    eprintln!("implausible page from the exec'd renderer: {summary:?}");
                    return 1;
                }
                if page.hit_regions.is_empty() {
                    eprintln!("the exec'd renderer shipped no hit-test geometry");
                    return 1;
                }
                let paths = loader.paths.lock().clone();
                for expected in ["/tile.png", "/page.css", "/face.ttf"] {
                    if !paths.iter().any(|p| p.ends_with(expected)) {
                        eprintln!("the exec'd renderer never requested {expected} (saw: {paths:?})");
                        return 1;
                    }
                }
                if cfg!(feature = "cairo-tiles") && tiles.is_empty() {
                    eprintln!("no tiles arrived from the exec'd renderer");
                    return 1;
                }
                println!(
                    "exec'd renderer rendered a {:.0}x{:.0} page: {} tiles, {} brokered requests",
                    summary.page_width,
                    summary.page_height,
                    tiles.len(),
                    paths.len()
                );
                0
            }
            Err(e) => {
                eprintln!("exec'd render failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the exec'd renderer exists only on Linux");
        2
    }
}

/// The engine-side wiring: `GosubEngine` itself spawns the fork server when
/// `security.renderer_process` is on, announces the tier, hands out the
/// handle, and tears it down at shutdown.
fn engine_renderer_process<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_config::settings::Setting;
        use gosub_engine::fork_server::protocol::ConfinementTier;
        use gosub_engine::GosubEngine;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;

        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("could not build a runtime: {e}");
                return 1;
            }
        };

        runtime.block_on(async move {
            let compositor = Arc::new(DefaultCompositor::default());
            // The engine runs the same configuration the child dispatch uses,
            // so its (static) font-system tier decides the renderer mechanism.
            let mut engine: GosubEngine<TileConfig<F>> =
                GosubEngine::new(None, Arc::new(NullBackend::new()), Arc::clone(&compositor));

            if let Err(e) = engine.settings().set("security.renderer_process", Setting::Bool(true)) {
                eprintln!("could not enable the renderer process: {e}");
                return 1;
            }

            let Ok(run) = engine.start() else {
                eprintln!("engine failed to start");
                return 1;
            };
            tokio::spawn(run);

            // What the engine starts depends on the configured font system's
            // static tier: `Full` gets a warmed fork server, `FontPathsReadable`
            // gets exec-per-render mode (no long-lived process at all).
            match F::confinement() {
                Confinement::Full => {
                    let Some(tier) = engine.renderer_process_tier() else {
                        eprintln!("the engine did not start a renderer fork server");
                        return 1;
                    };
                    println!("engine renderer process tier: {tier:?}");
                    if !matches!(tier, ConfinementTier::Full) {
                        eprintln!("expected the Full tier, got {tier:?}");
                        return 1;
                    }
                    let Some(server) = engine.renderer_process() else {
                        eprintln!("tier announced but no handle exposed");
                        return 1;
                    };
                    let outcome = server.lock().render_page(
                        "<html><body><p>Rendered through the engine's fork server.</p></body></html>",
                        "http://harness.invalid/",
                        "fork-harness-tab",
                        (1280.0, 720.0),
                        &gosub_interface::resource_loader::NoResourceLoader,
                        &Default::default(),
                        None,
                    );
                    match outcome {
                        Ok(page) => {
                            let (summary, tiles) = (page.summary, page.tiles);
                            if summary.page_height <= 0.0 || summary.paint_commands == 0 {
                                eprintln!("implausible page through the engine handle: {summary:?}");
                                return 1;
                            }
                            println!(
                                "engine-held fork server rendered a {:.0}x{:.0} page ({} tiles over shm)",
                                summary.page_width,
                                summary.page_height,
                                tiles.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("rendering through the engine handle failed: {e}");
                            return 1;
                        }
                    }
                }
                Confinement::FontPathsReadable => {
                    if engine.renderer_process_tier().is_some() {
                        eprintln!("a FontPathsReadable config must not spawn a fork server");
                        return 1;
                    }
                    println!("exec-per-render mode: no fork server spawned, renderers spawn per render");
                }
                Confinement::Unsupported(reason) => {
                    eprintln!("unexpected Unsupported tier: {reason}");
                    return 1;
                }
            }

            // The tab route: a real navigation whose frame is rendered
            // out-of-process - forked from the fork server (Full) or in a
            // throwaway exec'd renderer (FontPathsReadable). The tab worker
            // captures the document source, sends it out, and submits the
            // returned tiles to the compositor - the same code path an
            // embedder's window drives.
            {
                use gosub_engine::events::{NavigationEvent, TabCommand};
                use gosub_engine::storage::{
                    InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService,
                };
                use gosub_engine::zone::ZoneServices;
                use gosub_render_pipeline::render::backend::ExternalHandle;

                let Ok((port, _server)) = serve_once() else {
                    eprintln!("could not start the test server");
                    return 1;
                };
                let mut events = engine.subscribe_events();
                let services = ZoneServices {
                    storage: Arc::new(StorageService::new(
                        Arc::new(InMemoryLocalStore::new()),
                        Arc::new(InMemorySessionStore::new()),
                    )),
                    cookie_store: None,
                    cookie_jar: None,
                    partition_policy: PartitionPolicy::None,
                    places: None,
                };
                let Ok(mut zone) = engine.create_zone(None, services, None) else {
                    eprintln!("could not create a zone");
                    return 1;
                };
                let Ok(tab) = zone.create_tab(Default::default(), None).await else {
                    eprintln!("could not create a tab");
                    return 1;
                };
                let _ = tab
                    .send(TabCommand::SetViewport {
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 720,
                    })
                    .await;
                if tab.navigate(format!("http://127.0.0.1:{port}/")).await.is_err() {
                    eprintln!("navigate failed");
                    return 1;
                }
                let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;

                // Wait for the navigation, then for a frame to reach the
                // compositor - both on a clock.
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        eprintln!("timed out waiting for the navigation to finish");
                        return 1;
                    }
                    match tokio::time::timeout(remaining, events.recv()).await {
                        Ok(Ok(gosub_engine::events::EngineEvent::Navigation {
                            event: NavigationEvent::Finished { .. },
                            ..
                        })) => break,
                        Ok(Ok(gosub_engine::events::EngineEvent::Navigation {
                            event: NavigationEvent::Failed { error, .. },
                            ..
                        })) => {
                            eprintln!("navigation failed: {error}");
                            return 1;
                        }
                        Ok(Ok(_)) => continue,
                        _ => {
                            eprintln!("event channel closed or timed out");
                            return 1;
                        }
                    }
                }

                let frame = loop {
                    if tokio::time::Instant::now() >= deadline {
                        eprintln!("timed out waiting for a remotely rendered frame");
                        return 1;
                    }
                    match compositor.frame_for(tab.tab_id) {
                        Some(handle @ ExternalHandle::TileCache { .. }) => break handle,
                        _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
                    }
                };
                let ExternalHandle::TileCache { tiles, page_height, .. } = frame else {
                    eprintln!("expected a TileCache frame");
                    return 1;
                };
                if page_height <= 0.0 {
                    eprintln!("remotely rendered frame has no page height");
                    return 1;
                }
                if cfg!(feature = "cairo-tiles") && tiles.is_empty() {
                    eprintln!("remotely rendered frame carried no tiles");
                    return 1;
                }
                println!(
                    "tab frame rendered out-of-process: {} tiles, page height {page_height:.0}",
                    tiles.len()
                );

                // Hit testing on a remotely rendered page: the layer list is
                // process-local, so this can only work off the geometry the
                // renderer shipped. The served page is one big link, so a
                // move into it must raise HoverUrl with that href.
                let _ = tab.send(TabCommand::MouseMove { x: 50.0, y: 50.0 }).await;
                let hover_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
                let mut hovered: Option<String> = None;
                while tokio::time::Instant::now() < hover_deadline {
                    let remaining = hover_deadline.saturating_duration_since(tokio::time::Instant::now());
                    match tokio::time::timeout(remaining, events.recv()).await {
                        Ok(Ok(gosub_engine::events::EngineEvent::HoverUrl { url: Some(url), .. })) => {
                            hovered = Some(url);
                            break;
                        }
                        Ok(Ok(_)) => continue,
                        _ => break,
                    }
                }
                match hovered {
                    Some(url) if url.contains("example.test/target") => {
                        println!("hit test on the remotely rendered page found the link: {url}");
                    }
                    Some(url) => {
                        eprintln!("hovered an unexpected url: {url}");
                        return 1;
                    }
                    None => {
                        eprintln!("no HoverUrl for a remotely rendered page: hit testing is not working");
                        return 1;
                    }
                }

                engine.close_zone(zone).await;
            }

            if engine.shutdown().await.is_err() {
                eprintln!("engine shutdown failed");
                return 1;
            }
            0
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the renderer process exists only on Linux");
        2
    }
}

/// Answers a forked renderer's brokered subresource requests from memory,
/// recording them - the harness's stand-in for the engine's cookie-attaching
/// brokered loader. Serves an image, an external stylesheet (which declares a
/// layout-visible rule and an `@font-face`), and the font that face names.
#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
struct HarnessResourceLoader {
    served: std::sync::atomic::AtomicU64,
    /// Raw SFNT bytes served as `/face.ttf` (a real installed font, read
    /// broker-side where files are still reachable).
    font: Vec<u8>,
    /// Every path requested, in order - what the assertions read.
    paths: parking_lot::Mutex<Vec<String>>,
}

/// The stylesheet the renderer must fetch through the broker: one rule that
/// visibly moves layout (asserted via page height) and one `@font-face` whose
/// font must come back through the same channel.
#[cfg(target_os = "linux")]
const HARNESS_CSS: &str = r#"
    @font-face { font-family: "HarnessFace"; src: url("/face.ttf"); }
    .card { margin-top: 300px; font-family: "HarnessFace"; }
    h1:hover { background: #ff0000; }
"#;

#[cfg(target_os = "linux")]
impl gosub_interface::resource_loader::ResourceLoader for HarnessResourceLoader {
    fn load(
        &self,
        url: &url::Url,
    ) -> Result<gosub_interface::resource_loader::LoadedResource, gosub_interface::resource_loader::LoadError> {
        use gosub_interface::resource_loader::{LoadError, LoadedResource};
        self.served.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.paths.lock().push(url.path().to_string());
        match url.path() {
            path if path.ends_with("/tile.png") => Ok(LoadedResource {
                status: 200,
                content_type: Some("image/png".into()),
                body: bytes::Bytes::from_static(SAMPLE_PNG),
            }),
            path if path.ends_with("/page.css") => Ok(LoadedResource {
                status: 200,
                content_type: Some("text/css".into()),
                body: bytes::Bytes::from_static(HARNESS_CSS.as_bytes()),
            }),
            path if path.ends_with("/face.ttf") && !self.font.is_empty() => Ok(LoadedResource {
                status: 200,
                content_type: Some("font/ttf".into()),
                body: bytes::Bytes::from(self.font.clone()),
            }),
            _ => Err(LoadError::Failed(format!("harness does not serve {url}"))),
        }
    }
}

/// The whole phase-4 chain, end to end: the fork server consumes the
/// confinement answer and a forked renderer proves it was consumed correctly.
fn fork_server_roundtrip<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::ForkServer;
        use gosub_engine::fork_server::protocol::ConfinementTier;

        let mut server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        println!("fork server ready, tier: {:?}", server.confinement());

        match server.confinement() {
            ConfinementTier::Unsupported(reason) => {
                eprintln!("this font system cannot run isolated ({reason}); nothing to fork");
                server.shutdown();
                return 1;
            }
            // No zygote for this tier: renderers are exec'd fresh, so the
            // correct behaviour is a refusal that says exactly that.
            ConfinementTier::FontPathsReadable => {
                let reply = server.prove_shaping();
                server.shutdown();
                return match reply {
                    Err(e) if e.to_string().contains("no use for a fork server") => {
                        println!("fork refused as designed: {e}");
                        0
                    }
                    Err(e) => {
                        eprintln!("expected the designed refusal, got: {e}");
                        1
                    }
                    Ok(_) => {
                        eprintln!("a FontPathsReadable fork server should refuse to fork, but forked");
                        1
                    }
                };
            }
            ConfinementTier::Full => {}
        }

        match server.prove_shaping() {
            Ok((w, h)) => {
                if w <= 0.0 || h <= 0.0 {
                    eprintln!("forked renderer shaped an empty box ({w}x{h})");
                    return 1;
                }
                println!("forked renderer shaped {w:.1}x{h:.1} under its tier sandbox");
            }
            Err(e) => {
                eprintln!("fork proof failed: {e}");
                return 1;
            }
        }

        // The renderer role proper: the whole pipeline (parse → style →
        // layout → layering → tiling → paint) over a real page, in a fresh
        // forked renderer, single-threaded, under the tier sandbox. The page
        // exercises CSS (inline <style>), block flow, enough text that a
        // dead font system could not fake the numbers - and an image, which
        // the renderer cannot fetch: its request must come back out through
        // the fork server and be answered by the loader below.
        let html = r#"
            <html><head>
            <link rel="stylesheet" href="/page.css">
            <style>
                body { margin: 0; }
                h1 { font-size: 32px; }
                .card { padding: 16px; background: #eee; }
            </style></head>
            <body>
                <h1>Rendered in a forked renderer</h1>
                <div class="card"><p>Laid out, layered, tiled and painted under the
                renderer sandbox, shaping through fonts inherited copy-on-write from
                the fork server. No file was opened past this point.</p></div>
                <img src="/tile.png" width="64" height="64">
            </body></html>
        "#;
        // Real font bytes for `/face.ttf`, read broker-side (files are still
        // reachable here) - the renderer must receive them over the channel,
        // never from disk.
        let font_bytes = {
            use gosub_interface::font_system::FontQuery;
            let mut fonts = gosub_fontmanager::ParleyFontSystem::default();
            let _ = fonts.families();
            fonts
                .resolve(&FontQuery::new(&["sans-serif"]))
                .map(|resolved| resolved.blob.data.as_ref().as_ref().to_vec())
                .unwrap_or_default()
        };
        let loader = HarnessResourceLoader {
            font: font_bytes,
            ..Default::default()
        };
        let mut memory = gosub_engine::fork_server::client::TileMemory::default();
        // Filled from the first render's hit regions; the hover pass below
        // needs node ids, and only the renderer knows them.
        let mut hit_region_nodes: Vec<u64>;
        match server.render_page(
            html,
            "http://harness.invalid/index.html",
            "fork-harness-tab",
            (1280.0, 720.0),
            &loader,
            &memory,
            None,
        ) {
            Ok(page) => {
                let (summary, tiles, hit_regions) = (page.summary, page.tiles, page.hit_regions);
                if summary.page_height <= 0.0 || summary.painted_tiles == 0 || summary.paint_commands == 0 {
                    eprintln!("the forked renderer produced an implausible page: {summary:?}");
                    return 1;
                }
                // Hit-test geometry must cross with the pixels: without it a
                // remotely rendered page cannot answer what is under the
                // pointer. The page has an <a>, so a link region must exist.
                if hit_regions.is_empty() {
                    eprintln!("the forked renderer shipped no hit-test geometry");
                    return 1;
                }
                println!("received {} hit regions for the page", hit_regions.len());
                memory.replace_with(tiles.iter().map(|t| {
                    let (hash, kept) = t.keep();
                    (hash, kept)
                }));
                hit_region_nodes = hit_regions.iter().map(|r| r.node_id).collect();
                println!(
                    "forked renderer rendered a {:.0}x{:.0} page: {} layers, {} tiles painted, {} paint commands",
                    summary.page_width,
                    summary.page_height,
                    summary.layer_count,
                    summary.painted_tiles,
                    summary.paint_commands
                );
                // With a rasterizer compiled in, the pixels must arrive as
                // mapped shared memory - and be ink, not zeroes: these are
                // the renderer's own pages, validated and sealed.
                if cfg!(feature = "cairo-tiles") {
                    let inked: u64 = tiles
                        .iter()
                        .map(|tile| tile.pixels().iter().filter(|&&b| b != 0).count() as u64)
                        .sum();
                    if tiles.is_empty() || inked == 0 {
                        eprintln!("no ink arrived over shared memory ({} tiles)", tiles.len());
                        return 1;
                    }
                    println!(
                        "received {} tiles over shared memory ({inked} non-zero bytes, zero-copy)",
                        tiles.len()
                    );

                    // The consumer side, end to end: convert to the
                    // compositor's `CachedTile` shape (asserting the pixels
                    // are still the mapped pages, not a copy) and present a
                    // frame through the production composite loop.
                    use gosub_render_pipeline::render::tile_composite::{composite_tiles, TileTarget};
                    let mapped_ptrs: Vec<*const u8> = tiles.iter().map(|t| t.pixels().as_ptr()).collect();
                    let cached: Vec<_> = tiles.into_iter().map(|t| t.into_cached_tile()).collect();
                    for (cached_tile, mapped_ptr) in cached.iter().zip(&mapped_ptrs) {
                        if cached_tile.data.as_ptr() != *mapped_ptr {
                            eprintln!("a tile was copied on its way into the compositor");
                            return 1;
                        }
                    }

                    const BACKGROUND: u32 = 0xFF00_00FF; // opaque blue: absent from the page
                    let (vw, vh) = (1280usize, 720usize);
                    let mut frame = vec![BACKGROUND; vw * vh];
                    let mut target = TileTarget {
                        buf: &mut frame,
                        stride: vw,
                        origin_x: 0,
                        origin_y: 0,
                        width: vw,
                        height: vh,
                    };
                    composite_tiles(&cached, 1, (0.0, 0.0), &mut target);
                    let presented = frame.iter().filter(|&&px| px != BACKGROUND).count();
                    if presented == 0 {
                        eprintln!("compositing the mapped tiles painted nothing over the background");
                        return 1;
                    }
                    println!("composited a {vw}x{vh} frame from the mapped tiles ({presented} pixels changed)");
                } else if !tiles.is_empty() {
                    eprintln!("received tiles without a rasterizer compiled in?");
                    return 1;
                }
                // The subresource inversion: the confined renderer cannot
                // fetch, so its <img>, its <link> stylesheet, and the
                // @font-face that stylesheet declares must all have arrived
                // here as brokered requests.
                let paths = loader.paths.lock().clone();
                for expected in ["/tile.png", "/page.css", "/face.ttf"] {
                    if !paths.iter().any(|p| p.ends_with(expected)) {
                        eprintln!("the renderer never requested {expected} (saw: {paths:?})");
                        return 1;
                    }
                }
                // The external stylesheet must have *applied*, not merely
                // loaded: its 300px margin puts the page height beyond what
                // the inline styles alone produce.
                if summary.page_height < 300.0 {
                    eprintln!(
                        "page height {:.0} does not reflect the brokered stylesheet's 300px margin",
                        summary.page_height
                    );
                    return 1;
                }
                println!(
                    "served {} brokered requests ({} paths: img, stylesheet, web font); stylesheet applied",
                    loader.served.load(std::sync::atomic::Ordering::Relaxed),
                    paths.len()
                );
            }
            Err(e) => {
                eprintln!("rendering in a forked renderer failed: {e}");
                return 1;
            }
        }

        // Incrementality: render the same page again, this time telling the
        // renderer which tiles we kept. Nothing about the page changed, so
        // every tile must come back as unchanged - no rasterization, no
        // pixels, no file descriptors. Needs a rasterizer to have produced
        // tiles in the first place.
        if cfg!(feature = "cairo-tiles") {
            use gosub_engine::fork_server::client::PageTile;
            match server.render_page(
                html,
                "http://harness.invalid/index.html",
                "fork-harness-tab",
                (1280.0, 720.0),
                &loader,
                &memory,
                None,
            ) {
                Ok(page) => {
                    let fresh = page
                        .tiles
                        .iter()
                        .filter(|t| matches!(t, PageTile::Fresh { .. }))
                        .count();
                    let reused = page.tiles.len() - fresh;
                    if reused == 0 || fresh != 0 {
                        eprintln!("re-render shipped {fresh} fresh and {reused} reused tiles; expected all reused");
                        return 1;
                    }
                    println!("re-render reused all {reused} tiles: nothing rasterized, nothing shipped");
                }
                Err(e) => {
                    eprintln!("re-render failed: {e}");
                    return 1;
                }
            }
        }

        // Hover repaint, out of process: the renderer re-parses per render and
        // has no hover state of its own, so the broker tells it which node is
        // under the pointer. With the tile memory in play, only the tiles
        // whose painted content actually changed come back - hovering the
        // <h1> (which the brokered stylesheet gives a :hover background)
        // must repaint some tiles while reusing the rest.
        if cfg!(feature = "cairo-tiles") {
            use gosub_engine::fork_server::client::PageTile;
            let mut hovered_changed = 0usize;
            let mut nodes: Vec<u64> = std::mem::take(&mut hit_region_nodes);
            nodes.sort_unstable();
            nodes.dedup();
            for node in nodes {
                let Ok(page) = server.render_page(
                    html,
                    "http://harness.invalid/index.html",
                    "fork-harness-tab",
                    (1280.0, 720.0),
                    &loader,
                    &memory,
                    Some(node),
                ) else {
                    eprintln!("hover render failed for node {node}");
                    return 1;
                };
                let fresh = page
                    .tiles
                    .iter()
                    .filter(|t| matches!(t, PageTile::Fresh { .. }))
                    .count();
                if fresh > 0 {
                    hovered_changed += 1;
                    println!(
                        "hovering node {node} repainted {fresh} of {} tiles out of process",
                        page.tiles.len()
                    );
                }
            }
            if hovered_changed == 0 {
                eprintln!("no node produced a hover repaint; :hover is not reaching the renderer");
                return 1;
            }
        }

        // The streaming property: a page whose tile count exceeds any
        // process's file-descriptor limit (RLIMIT_NOFILE is 128 in the fork
        // server and its children) must still ship every tile - possible
        // only because each hop seals/relays/maps one fd at a time.
        if cfg!(feature = "cairo-tiles") {
            let tall = r#"<html><body style="margin:0">
                <div style="height: 12000px; background: #ddd">tall</div>
            </body></html>"#;
            match server.render_page(
                tall,
                "http://harness.invalid/tall.html",
                "fork-harness-tab",
                (1280.0, 720.0),
                &loader,
                &Default::default(),
                None,
            ) {
                Ok(page) => {
                    let (summary, tiles) = (page.summary, page.tiles);
                    if tiles.len() <= 128 {
                        eprintln!(
                            "expected a tall page to stream more tiles than an fd limit could buffer, got {}",
                            tiles.len()
                        );
                        return 1;
                    }
                    println!(
                        "streamed {} tiles for a {:.0}px-tall page (past any fd limit)",
                        tiles.len(),
                        summary.page_height
                    );
                }
                Err(e) => {
                    eprintln!("tall-page streaming render failed: {e}");
                    return 1;
                }
            }
        }
        server.shutdown();
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the fork server exists only on Linux");
        2
    }
}

/// Resident renderers, driven through the pool the engine uses: one process
/// per (zone, site) shared by that site's tabs, a cross-site navigation
/// moving a tab to another process, and the last tab leaving shutting the
/// process down.
fn renderer_lifecycle<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::ForkServer;
        use gosub_engine::fork_server::pool::RendererPool;
        use gosub_engine::fork_server::protocol::ConfinementTier;
        use gosub_engine::tab::TabId;
        use gosub_engine::zone::ZoneId;

        let server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        if !matches!(server.confinement(), ConfinementTier::Full) {
            eprintln!("renderer-lifecycle needs the Full tier, got {:?}", server.confinement());
            return 2;
        }
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)));
        let zone = ZoneId::new();
        let (tab_a, tab_b) = (TabId::new(), TabId::new());
        let loader = gosub_interface::resource_loader::NoResourceLoader;
        let memory = Default::default();

        let render = |site: &str, tab: TabId, html: &str| -> Result<i32, String> {
            let renderer = pool.renderer_for(zone, site, tab).map_err(|e| e.to_string())?;
            let mut renderer = renderer.lock();
            let page = renderer
                .navigate(
                    html,
                    &format!("{site}/"),
                    &tab.to_string(),
                    (1280.0, 720.0),
                    0.0,
                    &loader,
                    &memory,
                    None,
                )
                .map_err(|e| e.to_string())?;
            if page.summary.page_height <= 0.0 || page.summary.paint_commands == 0 {
                return Err(format!("implausible page: {:?}", page.summary));
            }
            Ok(renderer.pid())
        };

        // Two tabs, one site: one process, and it survives both renders.
        let pid_a = match render("https://one.test", tab_a, "<p>tab a, render 1</p>") {
            Ok(pid) => pid,
            Err(e) => {
                eprintln!("first render failed: {e}");
                return 1;
            }
        };
        let pid_b = match render("https://one.test", tab_b, "<p>tab b</p>") {
            Ok(pid) => pid,
            Err(e) => {
                eprintln!("second tab's render failed: {e}");
                return 1;
            }
        };
        let pid_a2 = match render("https://one.test", tab_a, "<p>tab a, render 2</p>") {
            Ok(pid) => pid,
            Err(e) => {
                eprintln!("re-render in the resident renderer failed: {e}");
                return 1;
            }
        };
        if pid_a != pid_b || pid_a != pid_a2 {
            eprintln!("same-site tabs should share one renderer: {pid_a} / {pid_b} / {pid_a2}");
            return 1;
        }
        let running = pool.snapshot();
        if running.len() != 1 || running[0].tabs != 2 {
            eprintln!("expected one renderer hosting two tabs, got {running:?}");
            return 1;
        }
        println!("two tabs on https://one.test share renderer pid {pid_a} (survived 3 renders)");

        // Tab A goes cross-site: a second process, while B keeps the first.
        let pid_other = match render("https://two.test", tab_a, "<p>tab a, elsewhere</p>") {
            Ok(pid) => pid,
            Err(e) => {
                eprintln!("cross-site render failed: {e}");
                return 1;
            }
        };
        if pid_other == pid_a {
            eprintln!("a cross-site navigation must land in a different renderer");
            return 1;
        }
        let running = pool.snapshot();
        if running.len() != 2 || running.iter().any(|r| r.tabs != 1) {
            eprintln!("expected two renderers with one tab each, got {running:?}");
            return 1;
        }
        println!("tab a moved to https://two.test: renderer pid {pid_other}; b still on {pid_b}");

        // The last tab leaving a site shuts its renderer down.
        pool.release(tab_b);
        let running = pool.snapshot();
        if running.len() != 1 || running[0].key.site != "https://two.test" {
            eprintln!("releasing the last tab should have shut https://one.test down, got {running:?}");
            return 1;
        }
        pool.release(tab_a);
        if !pool.snapshot().is_empty() {
            eprintln!(
                "releasing every tab should leave no renderer, got {:?}",
                pool.snapshot()
            );
            return 1;
        }
        println!("last tabs released: every renderer shut down");

        // A tab that returns after its renderer went away gets a new one.
        match render("https://one.test", tab_b, "<p>tab b is back</p>") {
            Ok(pid) if pid != pid_a => println!("returning tab got a fresh renderer, pid {pid}"),
            Ok(pid) => {
                eprintln!("a shut-down renderer's pid came back: {pid}");
                return 1;
            }
            Err(e) => {
                eprintln!("render after shutdown failed: {e}");
                return 1;
            }
        }

        pool.shutdown_all();
        pool.fork_server().lock().shutdown();
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the fork server exists only on Linux");
        2
    }
}

/// A resident renderer retains the page and rasterizes only around the
/// viewport: a tall page's first render ships a window of tiles rather than
/// the whole page, scrolling ships what came into the window, tiles left far
/// behind are evicted, and scrolling back ships them afresh.
fn renderer_scroll_window<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::{ForkServer, PageTile};
        use gosub_engine::fork_server::pool::RendererPool;
        use gosub_engine::fork_server::protocol::ConfinementTier;
        use gosub_engine::tab::TabId;
        use gosub_engine::zone::ZoneId;

        if !cfg!(feature = "cairo-tiles") {
            eprintln!("renderer-scroll-window needs a rasterizer (feature cairo-tiles)");
            return 2;
        }
        let server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        if !matches!(server.confinement(), ConfinementTier::Full) {
            eprintln!(
                "renderer-scroll-window needs the Full tier, got {:?}",
                server.confinement()
            );
            return 2;
        }
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)));
        let (zone, tab) = (ZoneId::new(), TabId::new());
        let loader = gosub_interface::resource_loader::NoResourceLoader;
        let mut memory = gosub_engine::fork_server::client::TileMemory::default();
        const PAGE: f64 = 12_000.0;
        const VP: (f64, f64) = (1280.0, 720.0);
        let tall =
            r#"<html><body style="margin:0"><div style="height: 12000px; background: #ddd">tall</div></body></html>"#;

        let renderer = match pool.renderer_for(zone, "https://tall.test", tab) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("could not get a renderer: {e}");
                return 1;
            }
        };
        let mut renderer = renderer.lock();
        let tab_name = tab.to_string();

        // First render, at the top: a window's worth of tiles, not the page's.
        let page = match renderer.navigate(tall, "https://tall.test/", &tab_name, VP, 0.0, &loader, &memory, None) {
            Ok(page) => page,
            Err(e) => {
                eprintln!("navigate failed: {e}");
                return 1;
            }
        };
        if page.summary.page_height < PAGE {
            eprintln!("expected a {PAGE}px page, got {:?}", page.summary);
            return 1;
        }
        let first = page.tiles.len();
        let rows_on_page = (PAGE / 256.0).ceil() as usize;
        if first == 0 || first >= rows_on_page * 3 {
            eprintln!("first render should ship a viewport window, not the page: {first} tiles");
            return 1;
        }
        let max_y = page
            .tiles
            .iter()
            .map(|t| match t {
                PageTile::Fresh { header, .. } | PageTile::Reused { header, .. } => header.page_y,
            })
            .fold(0.0, f64::max);
        if max_y > 3.0 * VP.1 {
            eprintln!("first render shipped a tile at y={max_y}, far outside the window");
            return 1;
        }
        memory.replace_with(page.tiles.iter().map(PageTile::keep));
        println!("first render shipped {first} tiles for a {PAGE}px page (lowest at y={max_y})");

        // Scroll to the middle: new tiles arrive, the top ones are evicted.
        let page = match renderer.scroll(&tab_name, 6000.0, &loader, &memory) {
            Ok(page) => page,
            Err(e) => {
                eprintln!("scroll failed: {e}");
                return 1;
            }
        };
        let fresh = page
            .tiles
            .iter()
            .filter(|t| matches!(t, PageTile::Fresh { .. }))
            .count();
        if fresh == 0 {
            eprintln!("scrolling into unrendered page must ship tiles");
            return 1;
        }
        if page.evicted.is_empty() {
            eprintln!("tiles three viewports behind must be evicted");
            return 1;
        }
        memory.apply_pass(&page.evicted, page.tiles.iter().map(PageTile::keep));
        println!("scroll to 6000: {fresh} tiles shipped, {} evicted", page.evicted.len());

        // Staying put ships nothing.
        let page = match renderer.scroll(&tab_name, 6000.0, &loader, &memory) {
            Ok(page) => page,
            Err(e) => {
                eprintln!("repeat scroll failed: {e}");
                return 1;
            }
        };
        if !page.tiles.is_empty() || !page.evicted.is_empty() {
            eprintln!(
                "a scroll that moved nowhere shipped {} tiles and evicted {}",
                page.tiles.len(),
                page.evicted.len()
            );
            return 1;
        }

        // Back to the top: the evicted tiles come back as fresh pixels.
        let page = match renderer.scroll(&tab_name, 0.0, &loader, &memory) {
            Ok(page) => page,
            Err(e) => {
                eprintln!("scroll back failed: {e}");
                return 1;
            }
        };
        let fresh = page
            .tiles
            .iter()
            .filter(|t| matches!(t, PageTile::Fresh { .. }))
            .count();
        if fresh == 0 {
            eprintln!("scrolling back over evicted tiles must ship them again");
            return 1;
        }
        println!(
            "scroll back to 0: {fresh} tiles re-shipped, {} evicted",
            page.evicted.len()
        );

        drop(renderer);
        pool.shutdown_all();
        pool.fork_server().lock().shutdown();
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the fork server exists only on Linux");
        2
    }
}

/// Hover on a retained page is a repaint, not a re-render: the renderer
/// restyles the hover chains and ships only the tiles the hovered element
/// covers - no parse, no layout - and nothing at all when the pointer stays.
fn renderer_hover<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::{ForkServer, PageTile};
        use gosub_engine::fork_server::pool::RendererPool;
        use gosub_engine::fork_server::protocol::ConfinementTier;
        use gosub_engine::tab::TabId;
        use gosub_engine::zone::ZoneId;

        if !cfg!(feature = "cairo-tiles") {
            eprintln!("renderer-hover needs a rasterizer (feature cairo-tiles)");
            return 2;
        }
        let server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        if !matches!(server.confinement(), ConfinementTier::Full) {
            eprintln!("renderer-hover needs the Full tier, got {:?}", server.confinement());
            return 2;
        }
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)));
        let (zone, tab) = (ZoneId::new(), TabId::new());
        let loader = gosub_interface::resource_loader::NoResourceLoader;
        let mut memory = gosub_engine::fork_server::client::TileMemory::default();
        let html = r#"<html><head><style>
            body { margin: 0; } p { height: 300px; }
            a { display: block; width: 200px; height: 40px; background: #ddd; }
            a:hover { background: #f00; }
        </style></head><body>
            <p>above</p><a href="/x">hover me</a><p>below</p><p>more</p><p>and more</p>
        </body></html>"#;

        let renderer = match pool.renderer_for(zone, "https://hover.test", tab) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("could not get a renderer: {e}");
                return 1;
            }
        };
        let mut renderer = renderer.lock();
        let tab_name = tab.to_string();
        let page = match renderer.navigate(
            html,
            "https://hover.test/",
            &tab_name,
            (1280.0, 720.0),
            0.0,
            &loader,
            &memory,
            None,
        ) {
            Ok(page) => page,
            Err(e) => {
                eprintln!("navigate failed: {e}");
                return 1;
            }
        };
        let first = page.tiles.len();
        // The link is the only 40px-tall box on the page.
        let Some(link) = page.hit_regions.iter().find(|r| (r.height - 40.0).abs() < 1.0) else {
            eprintln!("no hit region for the 40px link among {:?}", page.hit_regions);
            return 1;
        };
        let link_node = link.node_id;
        memory.replace_with(page.tiles.iter().map(PageTile::keep));
        println!("page rendered: {first} tiles, link is node {link_node}");

        let hover = |renderer: &mut gosub_engine::fork_server::client::ResidentRenderer,
                     memory: &mut gosub_engine::fork_server::client::TileMemory,
                     node: Option<u64>|
         -> Result<(usize, Vec<(String, u64)>), String> {
            let page = renderer
                .hover(&tab_name, node, &loader, memory)
                .map_err(|e| e.to_string())?;
            let fresh = page
                .tiles
                .iter()
                .filter(|t| matches!(t, PageTile::Fresh { .. }))
                .count();
            let timings = page.summary.timings_us.clone();
            memory.apply_pass(&page.evicted, page.tiles.iter().map(PageTile::keep));
            Ok((fresh, timings))
        };

        // Hovering the link repaints the tiles it covers, and nothing was laid out.
        let (fresh, timings) = match hover(&mut renderer, &mut memory, Some(link_node)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("hover failed: {e}");
                return 1;
            }
        };
        if fresh == 0 || fresh >= first {
            eprintln!("hovering the link should repaint a few tiles, got {fresh} of {first}");
            return 1;
        }
        if timings.iter().any(|(name, _)| name.starts_with("build.")) {
            eprintln!("a hover must not parse or lay out again: {timings:?}");
            return 1;
        }
        println!("hover on the link repainted {fresh} tile(s) with no layout");

        // Same node again: nothing to do.
        match hover(&mut renderer, &mut memory, Some(link_node)) {
            Ok((0, _)) => {}
            Ok((n, _)) => {
                eprintln!("hovering the same node again shipped {n} tiles");
                return 1;
            }
            Err(e) => {
                eprintln!("repeat hover failed: {e}");
                return 1;
            }
        }

        // Leaving it repaints the same tiles back.
        match hover(&mut renderer, &mut memory, None) {
            Ok((n, _)) if n > 0 => println!("leaving the link repainted {n} tile(s)"),
            Ok(_) => {
                eprintln!("leaving the link should repaint it un-hovered");
                return 1;
            }
            Err(e) => {
                eprintln!("un-hover failed: {e}");
                return 1;
            }
        }

        drop(renderer);
        pool.shutdown_all();
        pool.fork_server().lock().shutdown();
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the fork server exists only on Linux");
        2
    }
}

/// Debugging aid, not a test: render an arbitrary HTML file (argv[3], with an
/// optional base url in argv[4]) through the fork server, so a real-world page
/// that kills a forked renderer can be replayed headlessly. Subresources are
/// refused (`NoResourceLoader`), which real pages tolerate - the interesting
/// failures live in parse/style/layout/shaping/raster, not in the fetches.
fn render_file<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::ForkServer;
        use gosub_engine::fork_server::protocol::ConfinementTier;

        let Some(path) = std::env::args().nth(3) else {
            eprintln!("usage: isolation-harness render-file <font-backend> <page.html> [base-url]");
            return 2;
        };
        let html = match std::fs::read_to_string(&path) {
            Ok(html) => html,
            Err(e) => {
                eprintln!("could not read {path}: {e}");
                return 2;
            }
        };
        let base_url = std::env::args()
            .nth(4)
            .unwrap_or_else(|| "http://harness.invalid/".into());

        let mut server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        println!("fork server ready, tier: {:?}", server.confinement());
        if !matches!(server.confinement(), ConfinementTier::Full) {
            eprintln!("render-file needs the Full tier");
            server.shutdown();
            return 2;
        }

        let outcome = server.render_page(
            &html,
            &base_url,
            "render-file-tab",
            (1280.0, 720.0),
            &gosub_interface::resource_loader::NoResourceLoader,
            &Default::default(),
            None,
        );
        server.shutdown();
        match outcome {
            Ok(page) => {
                println!(
                    "forked renderer rendered {path}: {:.0}x{:.0}, {} tiles, {} paint commands",
                    page.summary.page_width,
                    page.summary.page_height,
                    page.tiles.len(),
                    page.summary.paint_commands
                );
                0
            }
            Err(e) => {
                eprintln!("forked render of {path} failed: {e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the fork server exists only on Linux");
        2
    }
}

/// `render-file` without the fork: the same pipeline over argv[3] in *this*
/// process under the renderer lockdown, so a SIGSYS can be caught by a
/// debugger with a full backtrace (`gdb --args isolation-harness
/// render-file-locked parley page.html`).
fn render_file_locked<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::renderer;

        let Some(path) = std::env::args().nth(3) else {
            eprintln!("usage: isolation-harness render-file-locked <font-backend> <page.html> [base-url]");
            return 2;
        };
        let html = match std::fs::read_to_string(&path) {
            Ok(html) => html,
            Err(e) => {
                eprintln!("could not read {path}: {e}");
                return 2;
            }
        };
        let base_url = std::env::args()
            .nth(4)
            .unwrap_or_else(|| "http://harness.invalid/".into());

        let mut fonts = F::default();
        let _ = fonts.families();
        match fonts.prepare_for_confinement() {
            Confinement::Full => {}
            other => {
                eprintln!("this scenario needs a fully-confinable font system, got {other:?}");
                return 2;
            }
        }
        // As in the fork server: fonts for SVG text pinned pre-lockdown, and
        // single-threaded fetches, since a confined renderer cannot create
        // threads.
        gosub_render_pipeline::common::media::SvgDecoder::pin_system_fonts();
        let media_store = std::sync::Arc::new(gosub_render_pipeline::common::media::MediaStore::new());
        media_store.set_synchronous_fetch(true);

        gosub_sandbox::lock_down_renderer();

        let shared: std::sync::Arc<parking_lot::Mutex<dyn FontSystem>> =
            std::sync::Arc::new(parking_lot::Mutex::new(fonts));
        let (summary, baked, _) = renderer::render_page::<TileConfig<F>>(
            renderer::PageRequest {
                html: &html,
                page_url: &base_url,
                viewport_width: 1280.0,
                viewport_height: 720.0,
                known_tiles: &Default::default(),
                hovered_node: None,
            },
            shared,
            media_store,
            std::sync::Arc::new(gosub_interface::resource_loader::NoResourceLoader),
        );
        println!(
            "rendered {path} under the renderer lockdown: {:.0}x{:.0}, {} tiles, {} paint commands",
            summary.page_width,
            summary.page_height,
            baked.len(),
            summary.paint_commands
        );
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the renderer lockdown exists only on Linux");
        2
    }
}

/// The follow-up question to the warm-up finding: a page can introduce a font at
/// any moment with `@font-face`, long after the sandbox is in place. Does that
/// need a file, and therefore a process that can open one?
fn webfont_under_lockdown<F: FontSystem + Default>() -> i32 {
    use gosub_interface::font_system::{FontQuery, TextStyle};

    let mut fonts = F::default();
    println!("font backend: {}", std::any::type_name::<F>());
    let _ = fonts.families();

    // Stand in for a downloaded font: bytes in hand, nothing else.
    let Ok(resolved) = fonts.resolve(&FontQuery::new(&["sans-serif"])) else {
        eprintln!("no resolvable font on this host to use as sample bytes");
        return 2;
    };
    let downloaded: Vec<u8> = resolved.blob.data.as_ref().as_ref().to_vec();
    if downloaded.is_empty() {
        eprintln!("resolved font carried no bytes; the control is broken");
        return 2;
    }
    println!("holding {} bytes of font data before lockdown", downloaded.len());

    // The renderer's documented sequence: prepare, confine, and only then let
    // content introduce fonts. Skipping the preparation here would test a
    // sequence no renderer runs - and shaping a web font still consults
    // fallback faces, which some backends (cosmic-text) load lazily per face.
    // Full lockdown only: backends answering a weaker tier are covered by the
    // font-readable scenarios.
    match fonts.prepare_for_confinement() {
        Confinement::Full => {}
        other => {
            eprintln!("this font system does not support full confinement: {other:?}");
            return 3;
        }
    }

    gosub_sandbox::lock_down_renderer();

    // Everything from here is what a renderer would do on encountering
    // `@font-face` mid-page.
    if let Err(e) = fonts.register_font(downloaded, Some("gosub-webfont-test")) {
        eprintln!("registering a web font under lockdown failed: {e:?}");
        return 1;
    }
    let (w, h) = fonts.measure(
        "Web font registered after the sandbox applied",
        &TextStyle::new(resolved.family.clone(), 24.0),
    );
    if w <= 0.0 || h <= 0.0 {
        eprintln!("shaping with the registered font produced an empty box ({w}x{h})");
        return 1;
    }
    println!("registered and shaped {w:.1}x{h:.1} under the renderer lockdown");
    0
}

/// Resident set size in MiB, from `/proc/self/statm` (pages), so the cost of a
/// strategy is a number rather than an impression. 0 where unavailable.
fn rss_mib() -> u64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    pages * 4096 / (1024 * 1024)
}

/// Decode the sample image repeatedly and report the wall-clock cost per image,
/// so the price of a process per decode is a measured number rather than a
/// guess. Count comes from argv[2], default 20.
fn decode_many() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::ImageDecoder;

    let count: u32 = std::env::args().nth(2).and_then(|a| a.parse().ok()).unwrap_or(20);

    let start = std::time::Instant::now();
    for _ in 0..count {
        if ProcessImageDecoder.decode(Some("image/png"), SAMPLE_PNG).is_err() {
            eprintln!("decode failed during timing run");
            return 1;
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{count} decodes in {:?} ({:.2} ms each)",
        elapsed,
        elapsed.as_secs_f64() * 1000.0 / f64::from(count)
    );
    0
}

/// Malformed input must come back as a refusal. This is the common case in the
/// wild - a truncated or hostile image - and it must not hang or crash the
/// broker.
fn decode_garbage() -> i32 {
    use gosub_engine::decoder_process::client::ProcessImageDecoder;
    use gosub_interface::media_decoder::ImageDecoder;

    // A PNG magic number followed by nonsense: it gets past a magic-byte sniff
    // and dies inside the decoder, which is where the danger actually lives.
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend(std::iter::repeat_n(0xA5, 4096));

    match ProcessImageDecoder.decode(Some("image/png"), &bytes) {
        Ok(other) => {
            eprintln!("garbage should not have decoded, got {other:?}");
            1
        }
        Err(_) => 0,
    }
}

/// An embedder that never dispatched: re-exec landed here, in `main`, rather
/// than in a component role. Spawning from this state would repeat the mistake
/// for every generation, so it must be refused.
fn guard() -> i32 {
    use gosub_engine::net::process::client::NetProcess;

    if !gosub_engine::child_process::is_child_process() {
        eprintln!("the guard scenario must be run with the child-role flag");
        return 2;
    }

    match NetProcess::spawn() {
        Ok(_) => {
            eprintln!("spawning should have been refused: an undispatched child must not spawn more");
            1
        }
        Err(e) => {
            // The message has to name the omission, or whoever hits this cannot
            // act on it.
            if e.to_string().contains("dispatch()") {
                0
            } else {
                eprintln!("refused, but not for the documented reason: {e}");
                1
            }
        }
    }
}

/// A one-shot HTTP server on an ephemeral port.
fn serve_once() -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
            BODY.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });

    Ok((port, handle))
}

/// The transport on its own: does a request survive the round trip through a
/// separate, sandboxed process and come back intact?
fn direct() -> i32 {
    use gosub_engine::net::process::client::NetProcess;
    use gosub_engine::net::process::protocol::FetchOutcome;

    let Ok((port, server)) = serve_once() else {
        eprintln!("could not start the test server");
        return 1;
    };

    let net = match NetProcess::spawn() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("could not spawn the network process: {e}");
            return 1;
        }
    };

    // `fetch` is async (the broker awaits it on its I/O runtime); the harness
    // has no runtime of its own, so give it a small one.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        eprintln!("could not start a runtime");
        return 1;
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let outcome = runtime.block_on(net.fetch(
        format!("http://127.0.0.1:{port}/"),
        "GET".into(),
        vec![("accept".into(), "text/html".into())],
        None,
        &cancel,
    ));
    net.shutdown();
    drop(server);

    match outcome {
        FetchOutcome::Ok { status, body, .. } => {
            if status != 200 {
                eprintln!("expected status 200, got {status}");
                return 1;
            }
            if body != BODY.as_bytes() {
                eprintln!(
                    "body did not survive the round trip: {:?}",
                    String::from_utf8_lossy(&body)
                );
                return 1;
            }
            0
        }
        FetchOutcome::Error(e) => {
            eprintln!("fetch through the network process failed: {e}");
            1
        }
    }
}

/// The wiring: with isolation on, does an ordinary navigation still resolve -
/// through the child process rather than an in-process fetcher?
fn engine() -> i32 {
    use gosub_config::settings::Setting;
    use gosub_engine::events::{EngineEvent, NavigationEvent};
    use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
    use gosub_engine::zone::ZoneServices;
    use gosub_engine::GosubEngine;
    use gosub_render_pipeline::render::backends::null::NullBackend;
    use gosub_render_pipeline::render::DefaultCompositor;

    let Ok((port, server)) = serve_once() else {
        eprintln!("could not start the test server");
        return 1;
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("could not build a runtime: {e}");
            return 1;
        }
    };

    let code = runtime.block_on(async move {
        let mut engine: GosubEngine = GosubEngine::new(
            None,
            Arc::new(NullBackend::new()),
            Arc::new(DefaultCompositor::default()),
        );

        // Read once when the I/O runtime starts, so it must be set before start().
        if let Err(e) = engine.settings().set("security.network_process", Setting::Bool(true)) {
            eprintln!("could not enable process isolation: {e}");
            return 1;
        }

        let mut events = engine.subscribe_events();
        let Ok(run) = engine.start() else {
            eprintln!("engine failed to start");
            return 1;
        };
        tokio::spawn(run);

        let services = ZoneServices {
            storage: Arc::new(StorageService::new(
                Arc::new(InMemoryLocalStore::new()),
                Arc::new(InMemorySessionStore::new()),
            )),
            cookie_store: None,
            cookie_jar: None,
            partition_policy: PartitionPolicy::None,
            places: None,
        };
        let Ok(mut zone) = engine.create_zone(None, services, None) else {
            eprintln!("could not create a zone");
            return 1;
        };
        let Ok(tab) = zone.create_tab(Default::default(), None).await else {
            eprintln!("could not create a tab");
            return 1;
        };
        if tab.navigate(format!("http://127.0.0.1:{port}/")).await.is_err() {
            eprintln!("navigate failed");
            return 1;
        }

        // The navigation is only meaningful if it *finished*: a failure would
        // also end the wait, so the variant is what is asserted.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                eprintln!("timed out waiting for the navigation to finish");
                return 1;
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Finished { .. },
                    ..
                })) => break,
                Ok(Ok(EngineEvent::Navigation {
                    event: NavigationEvent::Failed { error, .. },
                    ..
                })) => {
                    eprintln!("navigation failed: {error}");
                    return 1;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(e)) => {
                    eprintln!("event channel closed: {e}");
                    return 1;
                }
                Err(_) => {
                    eprintln!("timed out waiting for the navigation to finish");
                    return 1;
                }
            }
        }

        engine.close_zone(zone).await;
        let _ = engine.shutdown().await;
        0
    });

    drop(server);
    code
}
