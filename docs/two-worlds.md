# The two worlds: interface DOM vs. pipeline document

The most confusing architectural fact in this workspace, stated up front: **there are two parallel document/style models.** One is where parsing happens, the other is what the renderer consumes; they are different types in different crates and meet at exactly one adapter. This page explains what each world is, where the seam between them sits, and which code runs when.

> **Layout is no longer part of the split.** This page used to describe *three* duplicated concerns — document, style, *and* layout — because `gosub_taffy` implemented the interface-world layout traits and shipped its own `TaffyLayouter` alongside the pipeline's. Both are gone: the crate was removed, and with it the now-unimplemented `gosub_interface::layout` / `HasLayouter` traits. There is exactly one layouter, and it lives in the pipeline.

## World 1: the interface world (parsing and styles)

`gosub_interface` defines trait families for the engine's components, tied together by `ModuleConfiguration`: a config type `C` names concrete implementations as associated types, checked at compile time (no runtime registry). The configuration names exactly three components:

| Associated type | Trait | Implementation | Crate |
|-----------------|-------|----------------|-------|
| `Document` | `Document` | arena DOM | `gosub_html5` |
| `HtmlParser` | `Html5Parser` | spec-conformant tokenizer + tree builder | `gosub_html5` |
| `CssSystem` | `CssSystem` | tokenizer, parser, selector matcher, cascade | `gosub_css3` |

This is the world where **parsing happens**. When a tab loads a page, the engine parses HTML into `C::Document` and stylesheets into `C::CssSystem` stylesheets. Generic engine code only ever sees the traits.

`gosub_interface` hosts other contracts too — `FontSystem`, and the `RenderBackend` / `CompositorSink` backend contracts under `render/`. Those are *not* part of this split: they are shared by both sides and live in `gosub_interface` only so a config can name a backend without inverting the dependency direction. See [interface.md](interface.md).

## World 2: the pipeline world (rendering)

`gosub_render_pipeline` — everything documented under [render-pipeline/](render-pipeline/README.md) — has its **own, self-contained document model** under `src/common/document/`:

-   its own `Node` / `NodeType` / element data (`node.rs`);
-   its own HTML presentational-attribute handling (`presentation_hints.rs`, pending step 3b).

It no longer owns a style model. Style is `gosub_interface::style::ComputedStyle`, a typed struct with one field per property the pipeline reads, built once per element by the CSS crate. Neither the pipeline's `Node` nor its presentational hints implement `gosub_interface` traits, which is what makes the pipeline independently testable — its unit tests build documents from pipeline types directly, without an HTML parser or CSS engine in sight.

The pipeline also owns the **only** layouter in the workspace: `layouter/taffy.rs`'s `TaffyLayouter`, behind the pipeline-local `CanLayout` trait (documented in [render-pipeline/layout.md](render-pipeline/layout.md)), plus its `gosub_lattice` table bridge in `layouter/table.rs`. There is no counterpart in world 1 anymore, so a search for `TaffyLayouter` now has exactly one answer.

## The seam: `PipelineDocument` + `GosubDocumentAdapter`

The two worlds meet in one file: [`common/document/pipeline_doc.rs`](../crates/gosub_render_pipeline/src/common/document/pipeline_doc.rs).

**`PipelineDocument`** is the narrow trait the whole pipeline consumes: tree navigation (`root`/`children`/`parent`), node classification (`node_kind`/`tag_name`), and style — `computed_style(id) -> Arc<ComputedStyle>`, one struct in which every field already holds a value: the element's own, the one it inherited, or the property's initial. A `declared` set on the side answers the separate question of whether the element's own cascade said anything about a property, which a handful of readers need.

**`GosubDocumentAdapter<C: HasDocument>`** implements that trait over an `Arc<C::Document>` from world 1. It is where all the translation lives:

-   **Lazy computed styles**: on first `computed_style` for a node, the adapter runs world 1's CSS selector matching (`C::CssSystem`), caches the resulting property map, and converts it into a `ComputedStyle` against the parent's. Nothing is computed for nodes the pipeline never asks about. The map is kept alongside the struct: the cascade's own questions (custom-property scope, which origin won a declaration) and `getComputedStyle` still read it.
-   **Value translation**: `CssPropertyMap::computed_style` in gosub_css3 is the one place a CSS value becomes a typed field — colours, display keywords, lengths, grid track lists.
-   **Inline styles**: the `style=""` attribute is an ordinary stylesheet at inline specificity, cascaded by the CSS crate like any other.
-   **Generated content**: `::before` / `::after` have no DOM node, so the adapter mints *synthetic* `NodeId`s (bit-encoded: flag + role + owner id) and materializes pseudo-boxes lazily. The rest of the pipeline treats them as ordinary nodes.
-   **Invalidation**: `invalidate_style_for_nodes` / `clear_style_cache` let hover repaints re-run selector matching (`:hover`) for just the affected nodes.

The handoff happens in `gosub_engine`'s pipeline entry points (`crates/gosub_engine/src/engine/context.rs`): each rebuild wraps the parsed document in a fresh adapter and hands it to the pipeline's render-tree builder:

``` rust
let adapter = GosubDocumentAdapter::<C>::new(doc);   // world 1 in, world 2 out
let mut render_tree = RenderTree::new(Arc::new(adapter));
```

Everything upstream of that line is world 1; everything downstream is world 2.

## The full picture

``` text
        WORLD 1 (gosub_interface traits)          │            WORLD 2 (pipeline types)
                                                  │
  HTML  ──► gosub_html5 ──► C::Document ──┐       │
                                          ├──► GosubDocumentAdapter ──► RenderTree ──► layout
  CSS   ──► gosub_css3  ──► stylesheets ──┘       │   (PipelineDocument)              ──► … ──► pixels
                                                  │
                                                  │   layouter/taffy.rs (the only layouter)
```

## Duplications to be aware of

| Concept | World 1 | World 2 |
|---------|---------|---------|
| Document model | `C::Document` — the arena DOM | own `Node`/`NodeType` under `common/document/` |
| Style representation | `CssSystem` property maps; general | `ComputedStyle`, one typed field per property; closed, render-oriented |
| Node identity | `gosub_shared::node::NodeId` | same `NodeId`, plus synthetic pseudo-element ids |
| Layout | *(none)* | `layouter/taffy.rs`, `CanLayout`, `PipelineTableTree` |

Layout is listed only to head off an old assumption: `gosub_interface` has **no** layout contract. `layout.rs` (`Layouter<C>`, `LayoutTree<C>`, `LayoutNode`, `LayoutCache`, `Layout`, `TextLayout`/`HasTextLayout`) and the `HasLayouter` view were removed once `gosub_taffy` — their only implementor — went away. Adding or swapping a layouter today means implementing the pipeline's `CanLayout`.

## Why it is this way (and where it might go)

The trade is isolation versus duplication. Owning its document model lets the pipeline be developed and tested without routing every experiment through the full parse/cascade machinery, and gives the painter closed enums it can match on exhaustively; the cost is a conversion on every rebuild and one place to teach about any new CSS property — a field on `ComputedStyle` and the arm that fills it. The style half of the duplication is gone: the conversion lives in gosub_css3 and answers to `gosub_interface::style`, so there is no second property table, no second inheritance walk and no second parser of the `style` attribute.
