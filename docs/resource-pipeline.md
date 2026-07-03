# Resource pipelines

Where fetched bytes become typed assets. `crates/gosub_engine/src/engine/resource_pipeline/`
defines one pipeline per asset kind — HTML, CSS, JS, images, fonts — bundled into a
`ResourcePipelines<C>` struct that each [tab worker](zones-and-tabs.md) owns and hands to
the network response router.

```text
  fetch result ──► route_response_for (net/router.rs)      destination + UaPolicy decide:
                        │                                   render? download? DecisionRequired?
                        ▼
                ResourcePipelines<C>
                ├── HtmlPipeline   ──► EngineDocument (real DOM) + sub-resource discovery
                ├── CssPipeline    ──► stylesheet        (placeholder)
                ├── JsPipeline     ──► script source     (placeholder)
                ├── ImagePipeline  ──► image::DynamicImage
                └── FontPipeline   ──► font bytes        (placeholder)
```

Each pipeline is a small async trait with two entry points: `parse_stream` (a streaming
body plus the peek buffer the router already consumed for sniffing) and `parse_bytes`
(a fully buffered body). The router picks based on how the response arrived.

## The HTML pipeline (the real one)

`HtmlPipelineImpl` is the pipeline with actual machinery. `parse_main_document_stream`
(`src/html/parser.rs`) buffers the response body (capped, 1 MiB by default), parses it
into a real `EngineDocument<C>` DOM, and invokes an `on_discover` callback for every
sub-resource reference found — stylesheets (`<link rel="stylesheet">`), scripts
(`<script src>`), and images (`<img src>`).

The discovery callback is where early fetching happens: each `ResourceHint` becomes a
`FetchRequest` (initiator `Parser`, streaming, with the hint's priority) submitted straight
to the zone's I/O channel — so sub-resource downloads start as soon as the document parse
finds them, before layout ever asks for them.

Cancellation is hierarchical: every child fetch's token derives from the parent
navigation's `CancellationToken`. When the parse finishes (or fails, or the navigation is
cancelled), the parent token is cancelled and all in-flight child fetches die with it — no
orphaned downloads from an abandoned navigation. This behaviour is unit-tested in
`html.rs` (discovery submits three fetches for a three-resource page; all children are
cancelled after parse end).

Note the buffering: the *stream* interface is already in place end-to-end, but the parse
itself needs the full document first — the same non-incremental limitation described in
[html5.md](html5.md). When the parser becomes push-driven, discovery (and therefore
sub-resource fetching) moves earlier still: mid-download instead of after it.

## The others (mostly placeholders)

- **`ImagePipeline`** — decodes the body via the `image` crate (`with_guessed_format`) into
  a `DynamicImage`. Real, but note that images referenced from CSS/layout are *also*
  fetched via the render pipeline's `MediaStore` at layout time (see
  [render-pipeline/layout.md](render-pipeline/layout.md)); the parser-discovered fetch
  serves to warm the network layer early.
- **`CssPipeline`, `JsPipeline`, `FontPipeline`** — currently collect the body to a string
  (`DummyStylesheet` / `DummyJsDocument` / `DummyFont` are type aliases for `String`).
  The intended shape is chunk-feeding into the CSS parser / JS engine / font system; the
  traits exist so the router and tab worker don't change when the implementations land.

## Relation to routing and `UaPolicy`

The pipelines only see responses the router decided to *process*. `route_response_for`
consults the request's destination and the zone's `UaPolicy` (MIME sniffing, PDF viewer,
render-unknown-text-in-tab, download rules) first; responses that shouldn't be rendered go
to the download path or raise a `DecisionRequired` for the UA instead — see the navigation
flow in [zones-and-tabs.md](zones-and-tabs.md). The networking stack underneath
(`gosub_net`) is moving to the **gosub_sonar** project; the pipeline traits are the seam
the engine keeps regardless of where the fetcher lives.
