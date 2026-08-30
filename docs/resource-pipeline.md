# Resource pipelines

Where fetched bytes become typed assets. `crates/gosub_engine/src/engine/resource_pipeline/` defines one pipeline per asset kind --- HTML, CSS, JS, images, fonts --- bundled into a `ResourcePipelines<C>` struct that each [tab worker](zones-and-tabs.md) owns and hands to the network response router.

``` text
  fetch result ──► route_response_for (net/router.rs)      destination + UaPolicy decide:
                        │                                   render? download? DecisionRequired?
                        ▼
                ResourcePipelines<C>
                ├── HtmlPipeline   ──► EngineDocument (real DOM), or just its source
                ├── CssPipeline    ──► stylesheet        (placeholder)
                ├── JsPipeline     ──► script source     (placeholder)
                └── FontPipeline   ──► font bytes        (placeholder)
```

Images have no pipeline here: they are decoded where they are painted (the render
pipeline's `MediaStore`, through the decoder process when isolation is on), and the
router refuses to decode them.

Each pipeline is a small async trait with two entry points: `parse_stream` (a streaming body plus the peek buffer the router already consumed for sniffing) and `parse_bytes` (a fully buffered body). The router picks based on how the response arrived.

## The HTML pipeline (the real one)

`HtmlPipelineImpl` is the pipeline with actual machinery. `parse_main_document_stream` (`src/html/parser.rs`) buffers the response body (capped by `net.document.max_bytes`) and parses it into a real `EngineDocument<C>` DOM. When a renderer process will render the page, the pipeline runs in *source-only* mode instead: it keeps the bytes as text and never runs the HTML parser on page content in this process (see [process-isolation.md](process-isolation.md)).

Sub-resources are fetched by whoever consumes them, exactly once: the parser loads `<link rel="stylesheet">` through a `BrokeredLoader` bound to the tab (so the loads carry its identity and cookies), and images are fetched by the `MediaStore` at layout time. There is no parser-side prefetch --- an earlier regex "discovery" pass fetched every stylesheet, script and image up front and threw the bytes away, which doubled every stylesheet download and fetched scripts nothing consumes. A real preload wants a cache the consumers read from; that is the async resource pipeline plan, not a warm-up.

Cancellation is hierarchical: the loader's fetches derive their token from the parse's, and the parse cancels it when it ends (or fails, or the navigation is abandoned) --- no orphaned downloads. Unit-tested in `html.rs`: a page with a stylesheet, a script and an image causes exactly one fetch, the stylesheet's, cancelled after the parse.

Note the buffering: the *stream* interface is already in place end-to-end, but the parse itself needs the full document first --- the same non-incremental limitation described in [html5.md](html5.md).

## The others (mostly placeholders)

-   **`CssPipeline`, `JsPipeline`, `FontPipeline`** --- currently collect the body to a string (`DummyStylesheet` / `DummyJsDocument` / `DummyFont` are type aliases for `String`). The intended shape is chunk-feeding into the CSS parser / JS engine / font system; the traits exist so the router and tab worker don't change when the implementations land.

## Relation to routing and `UaPolicy`

The pipelines only see responses the router decided to *process*. `route_response_for` consults the request's destination and the zone's `UaPolicy` (MIME sniffing, PDF viewer, render-unknown-text-in-tab, download rules) first; responses that shouldn't be rendered go to the download path or raise a `DecisionRequired` for the UA instead --- see the navigation flow in [zones-and-tabs.md](zones-and-tabs.md). The networking stack underneath is the external **gosub-sonar** crate; the pipeline traits are the seam the engine keeps regardless of where the fetcher lives.
