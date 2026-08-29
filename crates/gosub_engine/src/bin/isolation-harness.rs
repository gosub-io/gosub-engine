//! Drives the network process end to end, from a binary that dispatches child
//! roles the way a real embedder does.

use gosub_interface::font_system::{Confinement, FontSystem};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const BODY: &str = "<html><head><title>through the net process</title></head>\
<body style=\"margin:0\"><a href=\"https://example.test/target\" \
style=\"display:block;width:400px;height:200px\">a link to hover</a></body></html>";

/// [`BODY`] with a long tail, for scrolling a remotely rendered page.
const TALL_BODY: &str = "<html><head><title>through the net process</title></head>\
<body style=\"margin:0\"><a href=\"https://example.test/target\" \
style=\"display:block;width:400px;height:200px\">a link to hover</a>\
<div style=\"height:12000px;background:#ddd\">tall</div></body></html>";

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
        "resolve" => resolve(),
        "vault" => vault(),
        "storage" => storage(),
        "engine-storage-service" => engine_storage_service(),
        "engine-cookie-vault" => engine_cookie_vault(),
        "stream" => stream(),
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
        "renderer-crash" => with_font_backend!(renderer_crash),
        "engine-renderer-crash" => with_font_backend!(engine_renderer_crash),
        "engine-renderer-slow-image" => with_font_backend!(engine_renderer_slow_image),
        "renderer-soak" => with_font_backend!(renderer_soak),
        "engine-soak" => with_font_backend!(engine_soak),
        "engine-stress" => with_font_backend!(engine_stress),
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
        Err(e) => {
            eprintln!("decode in a separate process failed: {e}");
            return 1;
        }
    }

    // Header parsing runs in the child too.
    match ProcessImageDecoder.dimensions(Some("image/png"), SAMPLE_PNG) {
        Ok((2, 2)) => {}
        other => {
            eprintln!("expected 2x2 from the decoder's header parse, got {other:?}");
            return 1;
        }
    }

    // SVG comes back rasterized at its intrinsic size: the tree never leaves
    // the child. A logo-sized SVG (with text) must produce pixels, not a
    // dead decoder.
    match ProcessImageDecoder.decode(Some("image/svg+xml"), SAMPLE_SVG) {
        Ok(BrokeredDecode::Raster(image)) if image.width > 1 && image.height > 1 => 0,
        Ok(BrokeredDecode::Raster(image)) => {
            eprintln!("an SVG rasterized to {}x{}", image.width, image.height);
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

                let Ok((port, _server)) = serve_once_with(TALL_BODY) else {
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

                // Scrolling a remotely rendered page: the first frame carried
                // only a window of the tall page; scrolling far past it must
                // bring more tiles, delivered by a pass that ran while frames
                // kept flowing - so a later frame has to hold more tiles.
                let _ = tab
                    .send(TabCommand::MouseScroll {
                        delta_x: 0.0,
                        delta_y: 6000.0,
                    })
                    .await;
                let scroll_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
                let grown = loop {
                    if tokio::time::Instant::now() >= scroll_deadline {
                        break None;
                    }
                    match compositor.frame_for(tab.tab_id) {
                        Some(ExternalHandle::TileCache {
                            tiles: now, scroll_y, ..
                        }) if now.len() > tiles.len() => {
                            break Some((now.len(), scroll_y));
                        }
                        _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
                    }
                };
                match grown {
                    Some((count, scroll_y)) if cfg!(feature = "cairo-tiles") => {
                        println!(
                            "scrolled to {scroll_y:.0}: frame grew from {} to {count} tiles via an async pass",
                            tiles.len()
                        );
                    }
                    Some(_) => {}
                    None if cfg!(feature = "cairo-tiles") => {
                        eprintln!("no frame with more tiles arrived after scrolling a tall remote page");
                        return 1;
                    }
                    None => {}
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
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)), None);
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
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)), None);
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
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)), None);
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

/// A resident renderer that dies is noticed on the next request and replaced:
/// the failed exchange marks it dead, the pool spawns a fresh one, and the
/// tab renders there.
fn renderer_crash<F: FontSystem + Default>() -> i32 {
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
            eprintln!("renderer-crash needs the Full tier, got {:?}", server.confinement());
            return 2;
        }
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)), None);
        let (zone, tab) = (ZoneId::new(), TabId::new());
        let loader = gosub_interface::resource_loader::NoResourceLoader;
        let memory = gosub_engine::fork_server::client::TileMemory::default();
        let html = "<html><body><p>a page that will lose its renderer</p></body></html>";
        let tab_name = tab.to_string();

        let first_pid = {
            let renderer = match pool.renderer_for(zone, "https://crash.test", tab) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("could not get a renderer: {e}");
                    return 1;
                }
            };
            let mut renderer = renderer.lock();
            if let Err(e) = renderer.navigate(
                html,
                "https://crash.test/",
                &tab_name,
                (1280.0, 720.0),
                0.0,
                &loader,
                &memory,
                None,
            ) {
                eprintln!("navigate failed: {e}");
                return 1;
            }
            renderer.pid()
        };
        println!("page rendered in renderer pid {first_pid}");

        if pool.crash_renderers_for_test("https://crash.test") != 1 {
            eprintln!("expected to crash exactly one renderer");
            return 1;
        }

        // The next exchange fails and marks the renderer dead ...
        {
            let renderer = match pool.renderer_for(zone, "https://crash.test", tab) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("renderer_for after the crash failed: {e}");
                    return 1;
                }
            };
            let mut renderer = renderer.lock();
            match renderer.scroll(&tab_name, 100.0, &loader, &memory) {
                Ok(_) => {
                    eprintln!("an exchange with a dead renderer should fail");
                    return 1;
                }
                Err(e) => println!("exchange with the dead renderer failed as expected: {e}"),
            }
            if !renderer.is_dead() {
                eprintln!("a failed exchange must mark the renderer dead");
                return 1;
            }
        }

        // ... and the request after that gets a fresh process that renders.
        let renderer = match pool.renderer_for(zone, "https://crash.test", tab) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("could not get a replacement renderer: {e}");
                return 1;
            }
        };
        let mut renderer = renderer.lock();
        if renderer.pid() == first_pid || renderer.is_dead() {
            eprintln!(
                "expected a fresh renderer, got pid {} (dead: {})",
                renderer.pid(),
                renderer.is_dead()
            );
            return 1;
        }
        match renderer.navigate(
            html,
            "https://crash.test/",
            &tab_name,
            (1280.0, 720.0),
            0.0,
            &loader,
            &memory,
            None,
        ) {
            Ok(page) if page.summary.paint_commands > 0 => {
                println!("replacement renderer pid {} rendered the page again", renderer.pid());
            }
            Ok(page) => {
                eprintln!("replacement rendered an implausible page: {:?}", page.summary);
                return 1;
            }
            Err(e) => {
                eprintln!("render in the replacement renderer failed: {e}");
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

/// The same crash through the engine: the tab's renderer dies, the embedder
/// hears `RendererCrashed`, and the tab comes back in a fresh process on its
/// next render - never in-process.
fn engine_renderer_crash<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_config::settings::Setting;
        use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
        use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
        use gosub_engine::zone::ZoneServices;
        use gosub_engine::GosubEngine;
        use gosub_interface::font_system::Confinement;
        use gosub_render_pipeline::render::backend::ExternalHandle;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;

        if !matches!(F::confinement(), Confinement::Full) {
            eprintln!("engine-renderer-crash needs a Full-tier font system");
            return 2;
        }
        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("could not build a runtime: {e}");
                return 1;
            }
        };
        runtime.block_on(async move {
            let compositor = Arc::new(DefaultCompositor::default());
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
            let Some(pool) = engine.renderer_pool().cloned() else {
                eprintln!("the engine did not start a renderer pool");
                return 1;
            };

            let Ok((port, _server)) = serve_once_with(TALL_BODY) else {
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

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
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
                    _ => {
                        eprintln!("timed out waiting for the navigation");
                        return 1;
                    }
                }
            }
            loop {
                if tokio::time::Instant::now() >= deadline {
                    eprintln!("timed out waiting for the first frame");
                    return 1;
                }
                if matches!(compositor.frame_for(tab.tab_id), Some(ExternalHandle::TileCache { .. })) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let site = "http://127.0.0.1";
            let Some(before) = pool.snapshot().into_iter().find(|r| r.key.site == site) else {
                eprintln!("no renderer for {site} in the pool: {:?}", pool.snapshot());
                return 1;
            };
            println!("tab rendered in renderer pid {}", before.pid);

            // Kill it, then give the tab a reason to talk to it.
            if pool.crash_renderers_for_test(site) != 1 {
                eprintln!("expected to crash one renderer");
                return 1;
            }
            let _ = tab
                .send(TabCommand::MouseScroll {
                    delta_x: 0.0,
                    delta_y: 6000.0,
                })
                .await;

            let crash_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                let remaining = crash_deadline.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(remaining, events.recv()).await {
                    Ok(Ok(EngineEvent::RendererCrashed {
                        site: crashed, tabs, ..
                    })) => {
                        if crashed != site || !tabs.contains(&tab.tab_id) {
                            eprintln!("RendererCrashed named the wrong site/tabs: {crashed} {tabs:?}");
                            return 1;
                        }
                        println!("embedder heard RendererCrashed for {crashed} ({} tab)", tabs.len());
                        break;
                    }
                    Ok(Ok(_)) => continue,
                    _ => {
                        eprintln!("no RendererCrashed event after killing the renderer");
                        return 1;
                    }
                }
            }

            // Recovery: a fresh process for the site, and the tab rendering in it.
            let recover_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
            let after = loop {
                if tokio::time::Instant::now() >= recover_deadline {
                    eprintln!("the tab never got a replacement renderer: {:?}", pool.snapshot());
                    return 1;
                }
                match pool.snapshot().into_iter().find(|r| r.key.site == site) {
                    Some(r) if r.pid != before.pid => break r,
                    _ => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
                }
            };
            println!("tab recovered in replacement renderer pid {}", after.pid);

            engine.close_zone(zone).await;
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

/// A render never waits for an image: a page whose image the server holds
/// back for seconds must still paint promptly, and paint again - without a
/// new navigation - once the image has arrived.
fn engine_renderer_slow_image<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_config::settings::Setting;
        use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
        use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
        use gosub_engine::zone::ZoneServices;
        use gosub_engine::GosubEngine;
        use gosub_interface::font_system::Confinement;
        use gosub_render_pipeline::render::backend::ExternalHandle;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;

        if !matches!(F::confinement(), Confinement::Full) {
            eprintln!("engine-renderer-slow-image needs a Full-tier font system");
            return 2;
        }
        const IMAGE_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
        let page = "<html><body style=\"margin:0\"><p>text</p><img src=\"/slow.png\" width=\"64\" height=\"64\"></body></html>";
        let Ok(port) = serve_routes(vec![
            ("/", "text/html", page.as_bytes().to_vec(), std::time::Duration::ZERO),
            ("/slow.png", "image/png", SAMPLE_PNG.to_vec(), IMAGE_DELAY),
        ]) else {
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
        runtime.block_on(async move {
            let compositor = Arc::new(DefaultCompositor::default());
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
            // The firehose says what each render was for.
            let mut firehose = gosub_engine::telemetry::subscribe();

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
            let started = tokio::time::Instant::now();
            if tab.navigate(format!("http://127.0.0.1:{port}/")).await.is_err() {
                eprintln!("navigate failed");
                return 1;
            }
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;

            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
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
                    _ => {
                        eprintln!("timed out waiting for the navigation");
                        return 1;
                    }
                }
            }
            let first_frame = loop {
                if tokio::time::Instant::now() >= deadline {
                    eprintln!("timed out waiting for the first frame");
                    return 1;
                }
                if matches!(compositor.frame_for(tab.tab_id), Some(ExternalHandle::TileCache { .. })) {
                    break started.elapsed();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            if first_frame >= IMAGE_DELAY {
                eprintln!("the first frame waited for the image: {first_frame:?} (image delay {IMAGE_DELAY:?})");
                return 1;
            }
            println!("first frame after {first_frame:?}, before the image could have arrived");

            // Then the image lands and the tab renders again - for that reason.
            let again = tokio::time::Instant::now() + IMAGE_DELAY + std::time::Duration::from_secs(5);
            let mut navigates = 0usize;
            let mut reasons: Vec<String> = Vec::new();
            let mut rerendered_for_media = false;
            while tokio::time::Instant::now() < again {
                let remaining = again.saturating_duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(remaining, firehose.recv()).await {
                    Ok(Ok(event)) => {
                        if event.kind == "tab.invalidate" {
                            let reason = event.data["reason"].as_str().unwrap_or("?").to_string();
                            if reason == "remote-media" {
                                rerendered_for_media = true;
                            }
                            reasons.push(reason);
                        }
                        if event.kind == "remote.navigate" {
                            navigates += 1;
                            if rerendered_for_media {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
            }
            println!("renders: {navigates} navigate(s); invalidations: {reasons:?}");
            if !rerendered_for_media {
                eprintln!("the tab never re-rendered for the late image (invalidations: {reasons:?})");
                return 1;
            }

            engine.close_zone(zone).await;
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

/// Standard base64 (RFC 4648, padded); enough for a `data:` URI in a test.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A long session in one resident renderer: many navigations with fresh
/// images each, many scroll passes, many tabs opened and closed. Memory must
/// level off rather than grow with every page, and the process must not
/// leave zombies behind when its tabs go.
fn renderer_soak<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::fork_server::client::{ForkServer, PageTile};
        use gosub_engine::fork_server::pool::{memory_of, RendererPool};
        use gosub_engine::fork_server::protocol::ConfinementTier;
        use gosub_engine::tab::TabId;
        use gosub_engine::zone::ZoneId;

        let rounds: usize = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(120);
        // Image edge in px per round (argv[4]); tiny images make the media
        // cache negligible, so any growth left is the pipeline's own.
        let image_px: usize = std::env::args().nth(4).and_then(|a| a.parse().ok()).unwrap_or(200);
        let server = match ForkServer::spawn() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("could not spawn the fork server: {e}");
                return 1;
            }
        };
        if !matches!(server.confinement(), ConfinementTier::Full) {
            eprintln!("renderer-soak needs the Full tier, got {:?}", server.confinement());
            return 2;
        }
        let fork_server_pid = server.pid().unwrap_or(0);
        let pool = RendererPool::new(Arc::new(parking_lot::Mutex::new(server)), None);
        let (zone, tab) = (ZoneId::new(), TabId::new());
        let loader = gosub_interface::resource_loader::NoResourceLoader;
        let site = "https://soak.test";
        let tab_name = tab.to_string();

        // A page with a unique inline image per round, so nothing is served
        // from a cache: every navigation decodes new pixels. 200x200 RGBA.
        let page_for = |round: usize| -> String {
            let mut png = Vec::new();
            {
                use image::ImageEncoder;
                let pixels: Vec<u8> = (0..image_px * image_px)
                    .flat_map(|i| [((i + round) % 251) as u8, (round % 200) as u8, (i % 7) as u8, 255])
                    .collect();
                let encoder = image::codecs::png::PngEncoder::new(&mut png);
                let _ = encoder.write_image(
                    &pixels,
                    image_px as u32,
                    image_px as u32,
                    image::ExtendedColorType::Rgba8,
                );
            }
            let data = base64(&png);
            format!(
                "<html><body style=\"margin:0\"><h1>round {round}</h1><img src=\"data:image/png;base64,{data}\" width=\"200\" height=\"200\">\
                 <div style=\"height:4000px;background:#eee\">{}</div></body></html>",
                "lorem ipsum ".repeat(round % 40 + 1)
            )
        };

        let mut pid = 0;
        let mut rss_after_warmup = 0u64;
        let warmup = (rounds / 5).max(5);
        for round in 0..rounds {
            let renderer = match pool.renderer_for(zone, site, tab) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("round {round}: no renderer: {e}");
                    return 1;
                }
            };
            let mut renderer = renderer.lock();
            if pid != 0 && renderer.pid() != pid {
                eprintln!(
                    "round {round}: the renderer was replaced (pid {pid} -> {})",
                    renderer.pid()
                );
                return 1;
            }
            pid = renderer.pid();
            let html = page_for(round);
            let mut memory = gosub_engine::fork_server::client::TileMemory::default();
            let page = match renderer.navigate(
                &html,
                &format!("{site}/{round}"),
                &tab_name,
                (1280.0, 720.0),
                0.0,
                &loader,
                &memory,
                None,
            ) {
                Ok(page) => page,
                Err(e) => {
                    eprintln!("round {round}: navigate failed: {e}");
                    return 1;
                }
            };
            memory.replace_with(page.tiles.iter().map(PageTile::keep));
            // Scroll down and back: passes, evictions, re-ships.
            for y in [1500.0, 3000.0, 0.0] {
                match renderer.scroll(&tab_name, y, &loader, &memory) {
                    Ok(page) => memory.apply_pass(&page.evicted, page.tiles.iter().map(PageTile::keep)),
                    Err(e) => {
                        eprintln!("round {round}: scroll failed: {e}");
                        return 1;
                    }
                }
            }
            if round + 1 == warmup {
                rss_after_warmup = memory_of(pid).map(|m| m.0).unwrap_or(0);
                println!(
                    "after {warmup} rounds: renderer pid {pid} rss {} MiB",
                    rss_after_warmup / 1024
                );
            } else if (round + 1) % 50 == 0 {
                let (rss, data) = memory_of(pid).unwrap_or((0, 0));
                println!(
                    "round {:>4}: rss {:>4} MiB  data {:>4} MiB",
                    round + 1,
                    rss / 1024,
                    data / 1024
                );
            }
        }
        let (rss_end, data_end) = memory_of(pid).unwrap_or((0, 0));
        println!(
            "after {rounds} rounds: rss {} MiB, data {} MiB (grew {} MiB since round {warmup})",
            rss_end / 1024,
            data_end / 1024,
            rss_end.saturating_sub(rss_after_warmup) / 1024
        );
        // Every round decodes a fresh 160 KB image and lays out a new page;
        // retained state must be replaced, not accumulated. 64 MiB of slack
        // covers allocator fragmentation and the tile cache.
        const MAX_GROWTH_KB: u64 = 64 * 1024;
        if rss_end.saturating_sub(rss_after_warmup) > MAX_GROWTH_KB {
            eprintln!(
                "renderer memory keeps growing: {} MiB -> {} MiB over {} rounds",
                rss_after_warmup / 1024,
                rss_end / 1024,
                rounds - warmup
            );
            return 1;
        }

        // Many tabs on one site, opened and closed: the process is shared,
        // survives, and is gone - reaped, not a zombie - once the last leaves.
        let extra: Vec<TabId> = (0..30).map(|_| TabId::new()).collect();
        for t in &extra {
            let renderer = match pool.renderer_for(zone, site, *t) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("could not add a tab: {e}");
                    return 1;
                }
            };
            let mut renderer = renderer.lock();
            if renderer
                .navigate(
                    "<p>tab</p>",
                    &format!("{site}/tab"),
                    &t.to_string(),
                    (640.0, 480.0),
                    0.0,
                    &loader,
                    &Default::default(),
                    None,
                )
                .is_err()
            {
                eprintln!("a tab's render failed");
                return 1;
            }
        }
        if pool.snapshot().first().map(|r| r.tabs) != Some(31) {
            eprintln!("expected 31 tabs on one renderer, got {:?}", pool.snapshot());
            return 1;
        }
        for t in &extra {
            pool.release(*t);
        }
        pool.release(tab);
        if !pool.snapshot().is_empty() {
            eprintln!("renderer should be gone after its last tab: {:?}", pool.snapshot());
            return 1;
        }
        // Give the exit a moment, then look for zombies among the fork
        // server's children (the pid-namespace anchor is a live child; fine).
        std::thread::sleep(std::time::Duration::from_millis(300));
        pool.fork_server().lock().reap_exited();
        let children = std::fs::read_to_string(format!("/proc/{fork_server_pid}/task/{fork_server_pid}/children"))
            .unwrap_or_default();
        let zombies: Vec<&str> = children
            .split_whitespace()
            .filter(|child| {
                std::fs::read_to_string(format!("/proc/{child}/stat"))
                    .ok()
                    .and_then(|stat| stat.rsplit(')').next().map(|rest| rest.trim_start().starts_with('Z')))
                    .unwrap_or(false)
            })
            .collect();
        if !zombies.is_empty() {
            eprintln!("zombie renderers under the fork server: {zombies:?}");
            return 1;
        }
        println!("30 tabs came and went on one renderer; no zombies under fork server {fork_server_pid}");

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

/// Not a test - a tool: the whole engine with every isolation setting on,
/// one tab navigating real sites (argv[3..], or a built-in image-heavy set)
/// in turn, reporting per site what it cost and what it took to render, and
/// at the end what the renderer processes hold. Exit 1 only if a renderer
/// crashed or a page could not be rendered out of process.
/// The firehose names pages in normalized form (`https://example.com/`).
fn same_page(reported: Option<&str>, asked: &str) -> bool {
    match (
        reported.and_then(|r| url::Url::parse(r).ok()),
        url::Url::parse(asked).ok(),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn engine_soak<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_config::settings::Setting;
        use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
        use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
        use gosub_engine::zone::ZoneServices;
        use gosub_engine::GosubEngine;
        use gosub_render_pipeline::render::backend::ExternalHandle;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;

        let mut urls: Vec<String> = std::env::args().skip(3).collect();
        if urls.is_empty() {
            urls = [
                "https://en.wikipedia.org/wiki/Main_Page",
                "https://www.bbc.com/news",
                "https://www.theverge.com",
                "https://www.nasa.gov",
                "https://commons.wikimedia.org/wiki/Main_Page",
                "https://news.ycombinator.com",
                "https://www.reddit.com/r/pics/",
                "https://unsplash.com",
            ]
            .into_iter()
            .map(String::from)
            .collect();
        }

        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("could not build a runtime: {e}");
                return 1;
            }
        };
        runtime.block_on(async move {
            let compositor = Arc::new(DefaultCompositor::default());
            let mut engine: GosubEngine<TileConfig<F>> =
                GosubEngine::new(None, Arc::new(NullBackend::new()), Arc::clone(&compositor));
            for key in [
                "security.network_process",
                "security.image_decoder_process",
                "security.renderer_process",
            ] {
                if let Err(e) = engine.settings().set(key, Setting::Bool(true)) {
                    eprintln!("could not enable {key}: {e}");
                    return 1;
                }
            }
            let Ok(run) = engine.start() else {
                eprintln!("engine failed to start");
                return 1;
            };
            tokio::spawn(run);
            let Some(pool) = engine.renderer_pool().cloned() else {
                eprintln!("renderer isolation did not start (font system tier?)");
                return 1;
            };
            let mut firehose = gosub_engine::telemetry::subscribe();
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
            let _ = tab.send(TabCommand::ResumeDrawing { fps: 30 }).await;

            println!(
                "{:<44} {:>7} {:>7} {:>5} {:>6} {:>8} {:>7} {:>7}  notes",
                "site", "nav ms", "frame", "tiles", "loads", "KiB", "rndr ms", "rss MiB"
            );
            let mut crashes = 0usize;
            let mut unrendered = 0usize;
            for url in &urls {
                let started = tokio::time::Instant::now();
                let mut notes: Vec<String> = Vec::new();
                if tab.navigate(url.clone()).await.is_err() {
                    println!("{url:<44} navigate refused");
                    continue;
                }
                // The navigation, on a clock.
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(40);
                let mut nav_ms: Option<u128> = None;
                // Where the navigation ended (redirects): the renderer reports that URL.
                let mut page_url = url.clone();
                loop {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        notes.push("navigation timed out".into());
                        break;
                    }
                    match tokio::time::timeout(remaining, events.recv()).await {
                        Ok(Ok(EngineEvent::Navigation {
                            event: NavigationEvent::Finished { url: finished, .. },
                            ..
                        })) => {
                            nav_ms = Some(started.elapsed().as_millis());
                            page_url = finished.to_string();
                            break;
                        }
                        Ok(Ok(EngineEvent::Navigation {
                            event: NavigationEvent::Failed { error, .. },
                            ..
                        })) => {
                            notes.push(format!("navigation failed: {error}"));
                            break;
                        }
                        Ok(Ok(EngineEvent::RendererCrashed { site, error, .. })) => {
                            crashes += 1;
                            notes.push(format!("RENDERER CRASHED ({site}: {error})"));
                        }
                        Ok(Ok(_)) => continue,
                        _ => break,
                    }
                }
                // The page's own out-of-process render, seen on the firehose
                // (`remote.navigate` names the page), then a grace period for
                // deferred images and the re-render they cause. The compositor
                // keeps showing the previous page until then, so its frame is
                // no signal on its own.
                let render_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
                let mut frame_ms: Option<u128> = None;
                let mut tiles = 0usize;
                let mut dump = std::env::var("GOSUB_SOAK_EVENTS").ok().and_then(|path| {
                    std::fs::OpenOptions::new().create(true).append(true).open(path).ok()
                });
                let (mut loads, mut bytes, mut renderer_us, mut navigates) = (0usize, 0usize, 0u64, 0usize);
                let mut grace_until: Option<tokio::time::Instant> = None;
                if nav_ms.is_some() {
                    loop {
                        let now = tokio::time::Instant::now();
                        let until = grace_until.unwrap_or(render_deadline);
                        if now >= until {
                            if grace_until.is_none() {
                                notes.push("no render within 60 s".into());
                            }
                            break;
                        }
                        let Ok(Ok(event)) = tokio::time::timeout(until - now, firehose.recv()).await else {
                            continue;
                        };
                        if let Some(file) = dump.as_mut() {
                            use std::io::Write as _;
                            let _ = writeln!(file, "{}", serde_json::json!({"ts_us": event.ts_us, "site": url, "kind": event.kind, "data": event.data}));
                        }
                        match event.kind.as_str() {
                            "net.load" => {
                                loads += 1;
                                bytes += event.data["bytes"].as_u64().unwrap_or(0) as usize;
                            }
                            "remote.navigate" | "remote.media" if same_page(event.data["url"].as_str(), &page_url) => {
                                navigates += 1;
                                if let Some(map) = event.data["renderer_us"].as_object() {
                                    renderer_us += map.values().filter_map(|v| v.as_u64()).sum::<u64>();
                                }
                                if frame_ms.is_none() {
                                    frame_ms = Some(started.elapsed().as_millis());
                                    grace_until = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(3));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                // Drain the engine events that arrived meanwhile (crashes).
                while let Ok(event) = events.try_recv() {
                    if let EngineEvent::RendererCrashed { site, error, .. } = event {
                        crashes += 1;
                        notes.push(format!("RENDERER CRASHED ({site}: {error})"));
                    }
                }
                if let Some(ExternalHandle::TileCache { tiles: t, .. }) = compositor.frame_for(tab.tab_id) {
                    tiles = t.len();
                }
                if nav_ms.is_some() && navigates == 0 {
                    unrendered += 1;
                    notes.push("NOT RENDERED OUT OF PROCESS".into());
                } else if navigates > 1 {
                    notes.push(format!("{navigates} renders"));
                }
                let rss = pool
                    .snapshot()
                    .into_iter()
                    .filter_map(|r| r.rss_kb)
                    .max()
                    .map(|kb| (kb / 1024).to_string())
                    .unwrap_or_else(|| "-".into());
                let short = if url.len() > 43 {
                    format!("{}…", &url[..42])
                } else {
                    url.clone()
                };
                println!(
                    "{short:<44} {:>7} {:>7} {tiles:>5} {loads:>6} {:>8} {:>7} {rss:>7}  {}",
                    nav_ms.map_or("-".into(), |v| v.to_string()),
                    frame_ms.map_or("-".into(), |v| v.to_string()),
                    bytes / 1024,
                    renderer_us / 1000,
                    notes.join("; ")
                );
            }

            println!();
            println!("renderers at the end:");
            for r in pool.snapshot() {
                println!(
                    "  pid {:>7}  {:<40} {} tab(s)  rss {} MiB  data {} MiB",
                    r.pid,
                    r.key.site,
                    r.tabs,
                    r.rss_kb.map_or(0, |kb| kb / 1024),
                    r.data_kb.map_or(0, |kb| kb / 1024)
                );
            }
            println!("crashes: {crashes}, pages not rendered out of process: {unrendered}");

            engine.close_zone(zone).await;
            let _ = engine.shutdown().await;
            i32::from(crashes > 0 || unrendered > 0)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the renderer process exists only on Linux");
        2
    }
}

/// Not a test - a tool: several tabs at once over real sites (the same site in
/// more than one tab on purpose), continuously navigating, scrolling, hovering,
/// closing and reopening for `argv[3]` seconds (default 120), logging every
/// action and every engine event as it happens, with a status line every few
/// seconds. `argv[4..]` replaces the built-in site list. Exit 1 on crashes.
fn engine_stress<F: FontSystem + Default>() -> i32 {
    println!("font backend: {}", std::any::type_name::<F>());
    #[cfg(target_os = "linux")]
    {
        use gosub_config::settings::Setting;
        use gosub_engine::events::{EngineEvent, NavigationEvent, TabCommand};
        use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
        use gosub_engine::tab::{TabHandle, TabId};
        use gosub_engine::zone::ZoneServices;
        use gosub_engine::GosubEngine;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;
        use std::collections::HashMap;

        let seconds: u64 = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(120);
        let mut sites: Vec<String> = std::env::args().skip(4).collect();
        if sites.is_empty() {
            sites = [
                "https://en.wikipedia.org/wiki/Main_Page",
                "https://www.bbc.com/news",
                "https://www.theverge.com",
                "https://www.nasa.gov",
                "https://commons.wikimedia.org/wiki/Main_Page",
                "https://news.ycombinator.com",
                "https://en.wikipedia.org/wiki/Cat",
                "https://www.bbc.com/sport",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                "https://www.rust-lang.org",
                "https://developer.mozilla.org/en-US/",
                "https://archive.org",
                "https://www.openstreetmap.org/about",
                "https://www.gnu.org",
                "https://lwn.net",
                "https://www.kernel.org",
            ]
            .into_iter()
            .map(String::from)
            .collect();
        }
        // Knobs beyond the positional args: how many tabs at once, and how
        // fast actions fire (the base of the 1x-3.4x random pause).
        let tabs_wanted: usize = std::env::var("GOSUB_STRESS_TABS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6)
            .max(1);
        let pace_ms: u64 = std::env::var("GOSUB_STRESS_PACE_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250)
            .max(10);

        // Deterministic per run, seedable; no need for a crate.
        let mut rng_state: u64 = std::env::var("GOSUB_STRESS_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let mut rng = move || {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            rng_state
        };

        let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("could not build a runtime: {e}");
                return 1;
            }
        };
        runtime.block_on(async move {
            let started = tokio::time::Instant::now();
            let stamp = move || format!("[{:>7.2}s]", started.elapsed().as_secs_f64());

            let compositor = Arc::new(DefaultCompositor::default());
            let mut engine: GosubEngine<TileConfig<F>> =
                GosubEngine::new(None, Arc::new(NullBackend::new()), Arc::clone(&compositor));
            for key in ["security.network_process", "security.image_decoder_process", "security.renderer_process"] {
                if let Err(e) = engine.settings().set(key, Setting::Bool(true)) {
                    eprintln!("could not enable {key}: {e}");
                    return 1;
                }
            }
            let Ok(run) = engine.start() else {
                eprintln!("engine failed to start");
                return 1;
            };
            tokio::spawn(run);
            let Some(pool) = engine.renderer_pool().cloned() else {
                eprintln!("renderer isolation did not start");
                return 1;
            };
            let mut firehose = gosub_engine::telemetry::subscribe();
            let mut events = engine.subscribe_events();
            println!(
                "{} engine up: {tabs_wanted} tabs over {} sites for {seconds}s (pace {pace_ms} ms; GOSUB_STRESS_TABS / GOSUB_STRESS_PACE_MS / GOSUB_STRESS_SEED to change); viewer: http://127.0.0.1:9090",
                stamp(),
                sites.len()
            );

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

            // Tabs by slot number, so the log can say "tab 3" rather than a uuid.
            let mut tabs: Vec<(usize, TabHandle)> = Vec::new();
            let mut names: HashMap<TabId, usize> = HashMap::new();
            let mut next_slot = 1usize;
            for _ in 0..tabs_wanted {
                let Ok(handle) = zone.create_tab(Default::default(), None).await else {
                    eprintln!("could not create a tab");
                    return 1;
                };
                let slot = next_slot;
                next_slot += 1;
                let _ = handle
                    .send(TabCommand::SetViewport {
                        x: 0,
                        y: 0,
                        width: 1280,
                        height: 720,
                    })
                    .await;
                let _ = handle.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                let site = sites[(rng() as usize) % sites.len()].clone();
                println!("{} tab {slot}: navigate {site}", stamp());
                let _ = handle.navigate(site).await;
                names.insert(handle.tab_id, slot);
                tabs.push((slot, handle));
            }

            let mut crashes = 0usize;
            let mut nav_ok = 0usize;
            let mut nav_failed = 0usize;
            let (mut loads, mut bytes, mut passes) = (0usize, 0usize, 0usize);
            let mut last_status = tokio::time::Instant::now();
            let deadline = started + std::time::Duration::from_secs(seconds);

            while tokio::time::Instant::now() < deadline {
                // One action on a random tab.
                let pick = (rng() as usize) % tabs.len();
                let (slot, handle) = (tabs[pick].0, tabs[pick].1.clone());
                let action = rng() % 100;
                match action {
                    0..=49 => {
                        let dy = ((rng() % 1200) as f32) - 300.0;
                        println!("{} tab {slot}: scroll {dy:+.0}", stamp());
                        let _ = handle.send(TabCommand::MouseScroll { delta_x: 0.0, delta_y: dy }).await;
                    }
                    50..=74 => {
                        let (x, y) = ((rng() % 1280) as f32, (rng() % 720) as f32);
                        println!("{} tab {slot}: hover ({x:.0},{y:.0})", stamp());
                        let _ = handle.send(TabCommand::MouseMove { x, y }).await;
                    }
                    75..=89 => {
                        let site = sites[(rng() as usize) % sites.len()].clone();
                        println!("{} tab {slot}: navigate {site}", stamp());
                        let _ = handle.navigate(site).await;
                    }
                    90..=94 => {
                        println!("{} tab {slot}: reload", stamp());
                        let _ = handle.send(TabCommand::Reload { ignore_cache: false }).await;
                    }
                    _ => {
                        println!("{} tab {slot}: close", stamp());
                        let id = handle.tab_id;
                        zone.close_tab(id).await;
                        names.remove(&id);
                        tabs.remove(pick);
                        if let Ok(handle) = zone.create_tab(Default::default(), None).await {
                            let slot = next_slot;
                            next_slot += 1;
                            let _ = handle
                                .send(TabCommand::SetViewport {
                                    x: 0,
                                    y: 0,
                                    width: 1280,
                                    height: 720,
                                })
                                .await;
                            let _ = handle.send(TabCommand::ResumeDrawing { fps: 30 }).await;
                            let site = sites[(rng() as usize) % sites.len()].clone();
                            println!("{} tab {slot}: open + navigate {site}", stamp());
                            let _ = handle.navigate(site).await;
                            names.insert(handle.tab_id, slot);
                            tabs.push((slot, handle));
                        }
                    }
                }

                // Let things happen, draining what the engine and the firehose say.
                let pause = std::time::Duration::from_millis(pace_ms + rng() % (pace_ms.saturating_mul(12) / 5).max(1));
                let until = tokio::time::Instant::now() + pause;
                loop {
                    let remaining = until.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    tokio::select! {
                        event = events.recv() => match event {
                            Ok(EngineEvent::Navigation { tab_id, event: NavigationEvent::Finished { .. } }) => {
                                nav_ok += 1;
                                println!("{} tab {}: navigation finished", stamp(), names.get(&tab_id).copied().unwrap_or(0));
                            }
                            Ok(EngineEvent::Navigation { tab_id, event: NavigationEvent::Failed { error, .. } }) => {
                                nav_failed += 1;
                                println!("{} tab {}: navigation FAILED: {error}", stamp(), names.get(&tab_id).copied().unwrap_or(0));
                            }
                            Ok(EngineEvent::RendererCrashed { site, tabs: affected, error, .. }) => {
                                crashes += 1;
                                let slots: Vec<usize> = affected.iter().filter_map(|t| names.get(t).copied()).collect();
                                println!("{} !!! RENDERER CRASHED for {site} (tabs {slots:?}): {error}", stamp());
                            }
                            Ok(_) => {}
                            Err(_) => {}
                        },
                        event = firehose.recv() => if let Ok(event) = event {
                            match event.kind.as_str() {
                                "net.load" => {
                                    loads += 1;
                                    bytes += event.data["bytes"].as_u64().unwrap_or(0) as usize;
                                    let outcome = event.data["outcome"].as_str().unwrap_or("");
                                    let ms = event.data["duration_us"].as_u64().unwrap_or(0) / 1000;
                                    if outcome != "ok" || ms > 2000 {
                                        println!(
                                            "{}   load {} ms {}: {} {}",
                                            stamp(),
                                            ms,
                                            event.data["url"].as_str().unwrap_or(""),
                                            outcome,
                                            event.data["error"].as_str().unwrap_or("")
                                        );
                                    }
                                }
                                "remote.navigate" | "remote.scroll" | "remote.hover" => {
                                    passes += 1;
                                    let ms = event.data["exchange_us"].as_u64().unwrap_or(0) / 1000;
                                    if ms > 1000 {
                                        let stages = event.data["renderer_us"]
                                            .as_object()
                                            .map(|m| {
                                                let mut parts: Vec<String> = m
                                                    .iter()
                                                    .map(|(k, v)| format!("{k} {}", v.as_u64().unwrap_or(0) / 1000))
                                                    .collect();
                                                parts.sort();
                                                parts.join(", ")
                                            })
                                            .unwrap_or_default();
                                        println!(
                                            "{}   slow {}: {ms} ms total ({stages}) ms, {} fresh tiles - {}",
                                            stamp(),
                                            event.kind,
                                            event.data["tiles_fresh"],
                                            event.data["url"].as_str().unwrap_or("")
                                        );
                                    }
                                }
                                _ => {}
                            }
                        },
                        _ = tokio::time::sleep(remaining) => break,
                    }
                }

                if last_status.elapsed() >= std::time::Duration::from_secs(5) {
                    last_status = tokio::time::Instant::now();
                    let renderers = pool.snapshot();
                    let rss_max = renderers.iter().filter_map(|r| r.rss_kb).max().unwrap_or(0) / 1024;
                    let rss_sum: u64 = renderers.iter().filter_map(|r| r.rss_kb).sum::<u64>() / 1024;
                    println!(
                        "{} === tabs {} | renderers {} (rss max {rss_max} MiB, total {rss_sum} MiB) | navs ok {nav_ok} failed {nav_failed} | loads {loads} ({} MiB) | passes {passes} | crashes {crashes}",
                        stamp(),
                        tabs.len(),
                        renderers.len(),
                        bytes / (1024 * 1024)
                    );
                    for r in &renderers {
                        println!(
                            "{}     pid {:>7} {:<38} {} tab(s) rss {} MiB",
                            stamp(),
                            r.pid,
                            r.key.site,
                            r.tabs,
                            r.rss_kb.map_or(0, |kb| kb / 1024)
                        );
                    }
                }
            }

            println!(
                "{} done: navs ok {nav_ok} failed {nav_failed} | loads {loads} ({} MiB) | passes {passes} | crashes {crashes}",
                stamp(),
                bytes / (1024 * 1024)
            );
            engine.close_zone(zone).await;
            let _ = engine.shutdown().await;
            i32::from(crashes > 0)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the renderer process exists only on Linux");
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

    match NetProcess::spawn(None) {
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

/// One route of [`serve_routes`]: path → (content type, body, delay before answering).
type Route = (&'static str, &'static str, Vec<u8>, std::time::Duration);

/// An HTTP server on an ephemeral port answering `routes` for as long as the
/// process lives, one connection at a time; unknown paths get a 404.
fn serve_routes(routes: Vec<Route>) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let route = routes.iter().find(|(p, ..)| *p == path);
            let (status, content_type, body): (&str, &str, &[u8]) = match route {
                Some((_, content_type, body, delay)) => {
                    std::thread::sleep(*delay);
                    ("200 OK", content_type, body)
                }
                None => ("404 Not Found", "text/plain", b"no such route"),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    Ok(port)
}

/// A one-shot HTTP server on an ephemeral port, serving [`BODY`].
fn serve_once() -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    serve_once_with(BODY)
}

/// A one-shot HTTP server on an ephemeral port, serving `body`.
fn serve_once_with(body: &'static str) -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    serve_once_bytes(body.as_bytes().to_vec(), "text/html")
}

/// A one-shot HTTP server on an ephemeral port, serving `body` in small
/// writes with pauses, the way a body arrives over a real network.
fn serve_once_bytes(body: Vec<u8>, content_type: &'static str) -> std::io::Result<(u16, std::thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        for chunk in body.chunks(32 * 1024) {
            if stream.write_all(chunk).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    Ok((port, handle))
}

/// A deterministic body larger than the ring window, so a stream wraps the
/// ring several times and every byte's position is checkable.
fn streamed_body() -> Vec<u8> {
    (0..(1024 * 1024 + 12345usize))
        .map(|i| (i.wrapping_mul(131) ^ (i >> 7)) as u8)
        .collect()
}

/// A zone built with a plain `FileLocalStore` gets its local storage served
/// by the storage process without the embedder asking: the setting's default.
fn engine_storage_service() -> i32 {
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::storage::{
            FileLocalStore, InMemorySessionStore, PartitionKey, PartitionPolicy, StorageService,
        };
        use gosub_engine::zone::ZoneServices;
        use gosub_engine::GosubEngine;
        use gosub_render_pipeline::render::backends::null::NullBackend;
        use gosub_render_pipeline::render::DefaultCompositor;

        let dir = std::env::temp_dir().join(format!("gosub-engine-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let Ok(store) = FileLocalStore::open(&dir) else {
            eprintln!("could not open the file store");
            return 1;
        };
        let Ok(origin) = url::Url::parse("https://app.test").map(|u| u.origin()) else {
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
            let _events = engine.subscribe_events();
            let Ok(run) = engine.start() else {
                eprintln!("engine failed to start");
                return 1;
            };
            tokio::spawn(run);
            let storage = Arc::new(StorageService::new(
                Arc::new(store),
                Arc::new(InMemorySessionStore::new()),
            ));
            let services = ZoneServices {
                storage: Arc::clone(&storage),
                cookie_store: None,
                cookie_jar: None,
                partition_policy: PartitionPolicy::None,
                places: None,
            };
            let zone = match engine.create_zone(None, services, None) {
                Ok(zone) => zone,
                Err(e) => {
                    eprintln!("could not create a zone: {e}");
                    return 1;
                }
            };
            // The embedder's own handle sees the routed store.
            let area = match storage.local_for(zone.id, &PartitionKey::None, &origin) {
                Ok(area) => area,
                Err(e) => {
                    eprintln!("no area: {e}");
                    return 1;
                }
            };
            if let Err(e) = area.set_item("k", "v") {
                eprintln!("set failed: {e}");
                return 1;
            }
            if area.get_item("k").as_deref() != Some("v") {
                eprintln!("get did not round-trip");
                return 1;
            }
            if !has_child_named("gosub-storage") {
                eprintln!(
                    "no gosub-storage child process: storage stayed in-process (children: {:?}, routed dir: {:?})",
                    child_names(),
                    storage.local_store().service_directory()
                );
                return 1;
            }
            println!("localStorage of a FileLocalStore zone is served by gosub-storage");
            let _ = engine.shutdown().await;
            0
        });
        let files = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        let _ = std::fs::remove_dir_all(&dir);
        if code == 0 && files == 0 {
            eprintln!("the service wrote nothing to the storage directory");
            return 1;
        }
        code
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the storage service is Linux-only");
        2
    }
}

/// The `comm` of every direct child of this process.
#[cfg(target_os = "linux")]
fn child_names() -> Vec<String> {
    let me = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let status = std::fs::read_to_string(entry.path().join("status")).ok()?;
            let mut comm = String::new();
            let mut ppid = String::new();
            for line in status.lines() {
                if let Some(v) = line.strip_prefix("Name:\t") {
                    comm = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("PPid:\t") {
                    ppid = v.trim().to_string();
                }
            }
            (ppid == me).then_some(comm)
        })
        .collect()
}

/// Whether this process has a direct child whose `comm` is `name`.
#[cfg(target_os = "linux")]
fn has_child_named(name: &str) -> bool {
    let me = std::process::id().to_string();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
        let mut comm = "";
        let mut ppid = "";
        for line in status.lines() {
            if let Some(v) = line.strip_prefix("Name:\t") {
                comm = v.trim();
            } else if let Some(v) = line.strip_prefix("PPid:\t") {
                ppid = v.trim();
            }
        }
        ppid == me && comm == name
    })
}

/// Storage service round trip, origin isolation, a refused oversize write,
/// persistence across a restart of the service.
fn storage() -> i32 {
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::storage::{LocalStore as _, PartitionKey, ServiceLocalStore};
        use gosub_engine::zone::ZoneId;

        let dir = std::env::temp_dir().join(format!("gosub-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let origin = |s: &str| url::Url::parse(s).map(|u| u.origin());
        let (Ok(a_origin), Ok(b_origin)) = (origin("https://a.test"), origin("https://b.test")) else {
            eprintln!("bad test origins");
            return 1;
        };
        let zone = ZoneId::new();

        let run = |expect_remote: bool| -> Result<(), String> {
            let store = ServiceLocalStore::new(&dir).map_err(|e| e.to_string())?;
            let a = store
                .area(zone, &PartitionKey::None, &a_origin)
                .map_err(|e| e.to_string())?;
            if expect_remote && !store.is_remote() {
                return Err("the storage service did not start; areas are in-process".into());
            }
            if a.get_item("k").is_none() {
                a.set_item("k", "1").map_err(|e| e.to_string())?;
                a.set_item("k2", "2").map_err(|e| e.to_string())?;
                let b = store
                    .area(zone, &PartitionKey::None, &b_origin)
                    .map_err(|e| e.to_string())?;
                if b.get_item("k").is_some() {
                    return Err("another origin must not see this origin's item".into());
                }
                if a.len() != 2 || a.get_item("k2").as_deref() != Some("2") {
                    return Err(format!("len/get wrong: len {} k2 {:?}", a.len(), a.get_item("k2")));
                }
                a.remove_item("k2").map_err(|e| e.to_string())?;
                if a.keys() != vec!["k".to_string()] {
                    return Err(format!("keys after remove: {:?}", a.keys()));
                }
                let huge = "v".repeat(gosub_engine::storage::file_store::MAX_VALUE_BYTES + 1);
                if a.set_item("huge", &huge).is_ok() {
                    return Err("an oversize value must be refused".into());
                }
                if a.get_item("k").as_deref() != Some("1") {
                    return Err("the service must survive a refused write".into());
                }
                println!("set/get/keys/remove/quota through the service ok");
            } else {
                if a.get_item("k").as_deref() != Some("1") || a.len() != 1 {
                    return Err(format!("state did not persist across a restart: {:?}", a.keys()));
                }
                a.clear().map_err(|e| e.to_string())?;
                if !a.is_empty() {
                    return Err("clear must empty the area".into());
                }
                println!("state persisted across a service restart");
            }
            store.shutdown();
            Ok(())
        };
        let outcome = run(true).and_then(|()| run(true));
        let _ = std::fs::remove_dir_all(&dir);
        match outcome {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the storage service is Linux-only");
        2
    }
}

/// The cookie vault on its own: a jar that forwards, the HttpOnly split, zone
/// partitioning, and persistence brokered through a real SQLite store - the
/// vault never opens the file, the broker does, from the snapshots it is sent.
#[cfg(target_os = "linux")]
fn line_channel(line: gosub_engine::cookie_vault::client::NetVaultLink) -> gosub_ipc::channel::Channel {
    line.0
}

fn vault() -> i32 {
    #[cfg(target_os = "linux")]
    {
        use gosub_engine::cookie_vault::client::{CookieVault, VaultCookieJar};
        use gosub_engine::cookie_vault::protocol::{CookieScope, SameSite};
        use gosub_engine::cookies::{CookieJar as _, CookieStoreHandle, SameSiteContext, SqliteCookieStore};
        use gosub_engine::zone::ZoneId;

        let (vault, _) = match CookieVault::spawn(false) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("could not spawn the vault: {e}");
                return 1;
            }
        };
        let vault = Arc::new(vault);
        let Ok(url) = url::Url::parse("https://example.test/app") else {
            eprintln!("bad test url");
            return 1;
        };
        let set_cookie = |values: &[&str]| {
            let mut headers = http::HeaderMap::new();
            for value in values {
                if let Ok(value) = value.parse() {
                    headers.append(http::header::SET_COOKIE, value);
                }
            }
            headers
        };

        // A zone with no store: in-memory in the vault.
        let zone = ZoneId::new();
        vault.open_zone(zone, None);
        let mut jar = VaultCookieJar::new(Arc::clone(&vault), zone);
        let headers = set_cookie(&["sid=abc; HttpOnly; Path=/", "theme=dark; Path=/"]);
        jar.store_response_cookies(&url, &headers, None);

        let attach = jar
            .get_request_cookies(&url, None, SameSiteContext::SameSite)
            .unwrap_or_default();
        if !(attach.contains("sid=abc") && attach.contains("theme=dark")) {
            eprintln!("the attachable set should hold both cookies, got {attach:?}");
            return 1;
        }
        let scope = CookieScope {
            ticket: 0,
            zone: zone.to_string(),
            top_level: None,
            samesite: SameSite::SameSite,
        };
        let visible = vault.get(scope.clone(), &url, true).unwrap_or_default();
        if visible.contains("sid=") || !visible.contains("theme=dark") {
            eprintln!("the document.cookie view must hide HttpOnly, got {visible:?}");
            return 1;
        }
        let other = ZoneId::new();
        vault.open_zone(other, None);
        let foreign = CookieScope {
            zone: other.to_string(),
            ..scope
        };
        if vault.get(foreign, &url, false).is_some() {
            eprintln!("another zone must not see this zone's cookies");
            return 1;
        }
        let listed = jar
            .get_all_cookies()
            .into_iter()
            .map(|(_, cookies)| cookies)
            .collect::<Vec<_>>()
            .join("; ");
        if !(listed.contains("sid=abc") && listed.contains("theme=dark")) {
            eprintln!("get_all_cookies should list both cookies, got {listed:?}");
            return 1;
        }
        println!("attach/visible/partition views correct");

        // The network process's line answers granted tickets only, and from
        // the grant's scope - not from what the line claims.
        let (net_vault, net_line) = match CookieVault::spawn(true) {
            Ok((v, Some(line))) => (Arc::new(v), line),
            _ => {
                eprintln!("could not spawn a vault with a network line");
                return 1;
            }
        };
        let Ok(mut net_link) = gosub_ipc::Endpoint::from_channel(line_channel(net_line)) else {
            eprintln!("could not open the network line");
            return 1;
        };
        net_vault.open_zone(zone, None);
        let mut net_jar = VaultCookieJar::new(Arc::clone(&net_vault), zone);
        net_jar.store_response_cookies(&url, &set_cookie(&["sid=abc; Path=/"]), None);
        use gosub_engine::cookie_vault::protocol::{FromVault, ToVault};
        let ask = |link: &mut gosub_ipc::Endpoint, scope: CookieScope| -> Option<String> {
            link.send(&ToVault::Get {
                tag: 7,
                scope,
                url: url.to_string(),
                visible_only: false,
            })
            .ok()?;
            match link.recv::<FromVault>().ok()? {
                FromVault::Cookies { header, .. } => header,
                _ => None,
            }
        };
        let claimed = CookieScope {
            ticket: 424242,
            zone: zone.to_string(),
            top_level: None,
            samesite: SameSite::SameSite,
        };
        if ask(&mut net_link, claimed.clone()).is_some() {
            eprintln!("the network line answered a ticket nobody granted");
            return 1;
        }
        if !net_vault.grant(&claimed) {
            eprintln!("the broker could not grant a ticket");
            return 1;
        }
        // Under the grant, the zone the line names is ignored: the grant's counts.
        let lying = CookieScope {
            zone: other.to_string(),
            ..claimed.clone()
        };
        let got = ask(&mut net_link, lying).unwrap_or_default();
        if !got.contains("sid=abc") {
            eprintln!("a granted ticket should answer from the grant's zone, got {got:?}");
            return 1;
        }
        net_vault.revoke(&claimed);
        std::thread::sleep(std::time::Duration::from_millis(100));
        if ask(&mut net_link, claimed).is_some() {
            eprintln!("the network line answered a revoked ticket");
            return 1;
        }
        println!("network line honours grants only");

        // A zone with a SQLite store: the vault's snapshots reach the file
        // through the broker, and a fresh store on the same file has them.
        let dir = std::env::temp_dir().join(format!("gosub-vault-{}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("no temp dir: {e}");
            return 1;
        }
        let path = dir.join("cookies.db");
        let store = match SqliteCookieStore::new(path.clone()) {
            Ok(s) => CookieStoreHandle::from(s),
            Err(e) => {
                eprintln!("could not open the sqlite store: {e}");
                return 1;
            }
        };
        let persisted = ZoneId::new();
        vault.open_zone(persisted, Some(store.clone()));
        let mut jar = VaultCookieJar::new(Arc::clone(&vault), persisted);
        let headers = set_cookie(&["durable=1; Path=/"]);
        jar.store_response_cookies(&url, &headers, None);
        // Reading back through the vault orders after the store (same link),
        // and the snapshot precedes the reply on the broker link.
        let _ = jar.get_request_cookies(&url, None, SameSiteContext::SameSite);
        store.persist_all();
        drop(store);
        let reopened = match SqliteCookieStore::new(path) {
            Ok(s) => CookieStoreHandle::from(s),
            Err(e) => {
                eprintln!("could not reopen the sqlite store: {e}");
                return 1;
            }
        };
        let back = reopened
            .jar_for(persisted)
            .and_then(|jar| jar.read().get_request_cookies(&url, None, SameSiteContext::SameSite))
            .unwrap_or_default();
        vault.shutdown();
        drop(reopened);
        let _ = std::fs::remove_dir_all(&dir);
        if !back.contains("durable=1") {
            eprintln!("the cookie did not reach the store through the broker, got {back:?}");
            return 1;
        }
        println!("brokered persistence reached sqlite: {back}");
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("the cookie vault is Linux-only");
        2
    }
}

/// The whole chain: engine with the vault and the network process on, a page
/// whose response sets an HttpOnly cookie, and a stylesheet the page loads
/// next. The second request must carry the cookie - which only the vault and
/// the network process ever handled - and the first must not.
fn engine_cookie_vault() -> i32 {
    use gosub_config::settings::Setting;
    use gosub_engine::storage::{InMemoryLocalStore, InMemorySessionStore, PartitionPolicy, StorageService};
    use gosub_engine::zone::ZoneServices;
    use gosub_engine::GosubEngine;
    use gosub_render_pipeline::render::backends::null::NullBackend;
    use gosub_render_pipeline::render::DefaultCompositor;
    use parking_lot::Mutex;

    let in_process = std::env::args().nth(2).as_deref() == Some("in-process");
    let seen: SeenRequests = Arc::new(Mutex::new(Vec::new()));
    let Ok(port) = serve_cookie_pages(Arc::clone(&seen)) else {
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
    let seen_after = Arc::clone(&seen);
    let code = runtime.block_on(async move {
        let mut engine: GosubEngine = GosubEngine::new(
            None,
            Arc::new(NullBackend::new()),
            Arc::new(DefaultCompositor::default()),
        );
        for (key, on) in [
            ("security.cookie_vault", true),
            ("security.network_process", !in_process),
            ("security.image_decoder_process", false),
            ("security.renderer_process", false),
        ] {
            if let Err(e) = engine.settings().set(key, Setting::Bool(on)) {
                eprintln!("could not set {key}: {e}");
                return 1;
            }
        }
        // The engine's event bus refuses to send without a subscriber.
        let _events = engine.subscribe_events();
        let Ok(run) = engine.start() else {
            eprintln!("engine failed to start");
            return 1;
        };
        tokio::spawn(run);
        if !engine.settings().get_bool("security.cookie_vault") {
            eprintln!("the vault did not start");
            return 1;
        }

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
        let mut zone = match engine.create_zone(None, services, None) {
            Ok(zone) => zone,
            Err(e) => {
                eprintln!("could not create a zone: {e}");
                return 1;
            }
        };
        let Ok(tab) = zone.create_tab(Default::default(), None).await else {
            eprintln!("could not create a tab");
            return 1;
        };
        if tab.navigate(format!("http://127.0.0.1:{port}/")).await.is_err() {
            eprintln!("navigate failed");
            return 1;
        }

        // Two requests: the page, then its stylesheet.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        while seen.lock().len() < 2 {
            if tokio::time::Instant::now() > deadline {
                eprintln!("timed out waiting for the stylesheet request; seen {:?}", seen.lock());
                return 1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let _ = engine.shutdown().await;
        0
    });
    if code != 0 {
        return code;
    }
    let seen = seen_after.lock().clone();
    println!("requests: {seen:?}");
    let first_clean = seen.first().is_some_and(|(_, cookie)| cookie.is_none());
    let second_has = seen
        .get(1)
        .is_some_and(|(path, cookie)| path == "/style.css" && cookie.as_deref().is_some_and(|c| c.contains("sid=abc")));
    if !first_clean || !second_has {
        eprintln!("the stylesheet request must carry the cookie the page set, and the page request must not");
        return 1;
    }
    println!(
        "cookie set by the page reached the next request through the vault{}",
        if in_process {
            " (in-process fetch)"
        } else {
            " and the network process"
        }
    );
    0
}

/// Every request a test server saw: its path and `Cookie` header.
type SeenRequests = Arc<parking_lot::Mutex<Vec<(String, Option<String>)>>>;

/// A server whose page sets an HttpOnly cookie and references a stylesheet;
/// every request's path and `Cookie` header are recorded in `seen`.
fn serve_cookie_pages(seen: SeenRequests) -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
            let cookie = request
                .lines()
                .find(|l| l.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("cookie:")))
                .map(|l| l[7..].trim().to_string());
            seen.lock().push((path.clone(), cookie));
            let (content_type, extra, body): (&str, &str, &str) = if path == "/style.css" {
                ("text/css", "", "body { color: rgb(1, 2, 3); }")
            } else {
                (
                    "text/html",
                    "Set-Cookie: sid=abc; HttpOnly; Path=/\r\n",
                    "<html><head><link rel=\"stylesheet\" href=\"/style.css\"></head><body>vaulted</body></html>",
                )
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
    Ok(port)
}

/// A streamed body through the network process: the head comes back in-band,
/// the ring fd right behind it, and the bytes arrive through the ring as the
/// child produces them - the whole body never sits in a message.
fn stream() -> i32 {
    use gosub_engine::net::process::client::NetProcess;
    use gosub_engine::net::process::protocol::FetchOutcome;

    let expected = streamed_body();
    let Ok((port, server)) = serve_once_bytes(expected.clone(), "application/octet-stream") else {
        eprintln!("could not start the test server");
        return 1;
    };
    let net = match NetProcess::spawn(None) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("could not spawn the network process: {e}");
            return 1;
        }
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        eprintln!("could not start a runtime");
        return 1;
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let out = gosub_engine::net::process::client::Outbound {
        streaming: true,
        ..gosub_engine::net::process::client::Outbound::get(format!("http://127.0.0.1:{port}/"))
    };
    let reply = runtime.block_on(net.fetch(out, &cancel));
    let result = match reply.outcome {
        FetchOutcome::Streaming { status, peek, .. } => {
            let Some(ring) = reply.ring else {
                eprintln!("streamed head arrived without its ring fd");
                net.shutdown();
                return 1;
            };
            #[cfg(target_os = "linux")]
            {
                let mut consumer = match gosub_ipc::ring::RingConsumer::open(ring) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("could not open the ring: {e}");
                        net.shutdown();
                        return 1;
                    }
                };
                let mut body = peek.clone();
                let mut buf = [0u8; 4096];
                loop {
                    match consumer.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => body.extend_from_slice(&buf[..n]),
                        Err(e) => {
                            eprintln!("ring read failed: {e}");
                            net.shutdown();
                            return 1;
                        }
                    }
                }
                println!("streamed {} bytes ({} peeked), status {status}", body.len(), peek.len());
                (status == 200 && body == expected).then_some(()).ok_or_else(|| {
                    format!(
                        "body did not survive the ring ({} of {} bytes)",
                        body.len(),
                        expected.len()
                    )
                })
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = ring;
                Err("streaming is Linux-only".to_string())
            }
        }
        FetchOutcome::Ok { .. } => Err("expected a streamed reply, got a buffered one".into()),
        FetchOutcome::Error(e) => Err(format!("fetch failed: {e}")),
    };
    net.shutdown();
    drop(server);
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Real hostname resolution inside the sandboxed network process. `127.0.0.1`
/// never reaches NSS, which is how two syscall denials (`mmap(PROT_EXEC)` from
/// `dlopen`ing NSS modules, `sendmmsg` from the resolver) survived every test
/// until an example hit a live URL. A reserved `.invalid` name exercises the
/// whole resolver path without needing the network: the fetch must fail, and
/// the process must *survive* it and still serve. The strict fetcher (a
/// subresource of a public page) must then refuse the loopback test server,
/// and the permissive one must still reach it.
fn resolve() -> i32 {
    use gosub_engine::net::process::client::NetProcess;
    use gosub_engine::net::process::protocol::FetchOutcome;

    let Ok((port, server)) = serve_once() else {
        eprintln!("could not start the test server");
        return 1;
    };
    let net = match NetProcess::spawn(None) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("could not spawn the network process: {e}");
            return 1;
        }
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        eprintln!("could not start a runtime");
        return 1;
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    let fetch = |url: String, refuse_private: bool| {
        let out = gosub_engine::net::process::client::Outbound {
            refuse_private,
            ..gosub_engine::net::process::client::Outbound::get(url)
        };
        runtime.block_on(net.fetch(out, &cancel)).outcome
    };

    // 1. A name that cannot exist (RFC 2606), through the permissive fetcher.
    match fetch("http://gosub-hostname-probe.invalid/".into(), false) {
        FetchOutcome::Ok { status, .. } | FetchOutcome::Streaming { status, .. } => {
            eprintln!("a .invalid name must not resolve, got status {status}");
            net.shutdown();
            return 1;
        }
        FetchOutcome::Error(e) => println!("resolution failed as it should: {e}"),
    }

    // 2. The strict fetcher classifies the loopback literal at the hop.
    match fetch(format!("http://127.0.0.1:{port}/"), true) {
        FetchOutcome::Ok { .. } | FetchOutcome::Streaming { .. } => {
            eprintln!("the strict fetcher reached loopback");
            net.shutdown();
            return 1;
        }
        FetchOutcome::Error(e) if e.contains("blocked") || e.contains("policy") => {
            println!("strict fetcher refused loopback: {e}");
        }
        FetchOutcome::Error(e) => {
            eprintln!("strict fetcher failed for the wrong reason: {e}");
            net.shutdown();
            return 1;
        }
    }

    // 3. The process is still alive and serving.
    let outcome = fetch(format!("http://127.0.0.1:{port}/"), false);
    net.shutdown();
    drop(server);
    match outcome {
        FetchOutcome::Ok { status: 200, body, .. } if body == BODY.as_bytes() => {
            println!("network process survived resolution and still serves");
            0
        }
        other => {
            eprintln!("the network process did not survive: {other:?}");
            1
        }
    }
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

    let net = match NetProcess::spawn(None) {
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
    let out = gosub_engine::net::process::client::Outbound {
        headers: vec![("accept".into(), "text/html".into())],
        ..gosub_engine::net::process::client::Outbound::get(format!("http://127.0.0.1:{port}/"))
    };
    let outcome = runtime.block_on(net.fetch(out, &cancel)).outcome;
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
        FetchOutcome::Streaming { .. } => {
            eprintln!("a buffered request came back streamed");
            1
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
