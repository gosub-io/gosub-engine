# The two worlds: interface DOM vs. pipeline document

The most confusing architectural fact in this workspace, stated up front: **there are two
parallel document/style/layout stacks.** Both wrap the Taffy layout engine, both bridge
table layout to `gosub_lattice`, and both even have a struct named `TaffyLayouter`. They
are different types in different crates, and only one of them is on the live rendering
path. This page explains what each world is, where the seam between them sits, and which
code runs when.

## World 1: the interface world (parsing and styles)

`gosub_interface` defines trait families for the engine's components — `Document`,
`Html5Parser`, `CssSystem`, `Layouter` — tied together by `ModuleConfiguration`: a config
type `C` names concrete implementations as associated types, checked at compile time (no
runtime registry). The implementations:

| Trait | Implementation | Crate |
|---|---|---|
| `Document`, `Html5Parser` | DOM + spec-conformant HTML5 parser | `gosub_html5` |
| `CssSystem` | tokenizer, parser, selector matcher, cascade | `gosub_css3` |
| `Layouter` | `gosub_taffy::TaffyLayouter` | `gosub_taffy` |

This is the world where **parsing happens**. When a tab loads a page, the engine parses
HTML into `C::Document` and stylesheets into `C::CssSystem` stylesheets. Generic engine
code only ever sees the traits.

`gosub_taffy` is the interesting resident: its `LayoutDocument<C>` implements Taffy's own
traits (`TraversePartialTree`, `LayoutPartialTree`, `CacheTree`) *directly over the
interface-world DOM*, so Taffy walks the real document without an intermediate tree. It is
architecturally elegant — and currently **dormant**: `gosub_engine` does not depend on it,
and nothing in the workspace uses it on the rendering path. It survives as a re-export in
the root crate's prelude (`src/prelude.rs`).

## World 2: the pipeline world (rendering)

`gosub_render_pipeline` — everything documented under [render-pipeline/](render-pipeline/README.md) —
has its **own, self-contained document model** under `src/common/document/`:

- its own `Node` / `NodeType` / element data (`node.rs`);
- its own style model (`style.rs`): a closed `StyleProperty` enum and `Value` type with
  interned keywords, per-property metadata (inherited? initial value?), and its own
  inheritance + `em`/`rem` resolution;
- its own layouter (`layouter/taffy.rs` — the *other* `TaffyLayouter`, behind the
  pipeline-local `CanLayout` trait, documented in [render-pipeline/layout.md](render-pipeline/layout.md));
- its own table bridge to `gosub_lattice` (`layouter/table.rs`).

None of these types implement `gosub_interface` traits. The pipeline's style model is small
and rendering-oriented (exactly the properties the painter needs, as plain enums), where
the css3 world's property maps are fully general. This is what makes the pipeline
independently testable — its unit tests build documents from pipeline types directly,
without an HTML parser or CSS engine in sight.

## The seam: `PipelineDocument` + `GosubDocumentAdapter`

The two worlds meet in one file:
[`common/document/pipeline_doc.rs`](../crates/gosub_render_pipeline/src/common/document/pipeline_doc.rs).

**`PipelineDocument`** is the narrow trait the whole pipeline consumes: tree navigation
(`root`/`children`/`parent`), node classification (`node_kind`/`tag_name`), and styles —
`get_own_style(id, prop) -> Option<Value>` plus a provided `get_style` that layers
inheritance, initial values, and `em`/`rem` → px resolution on top.

**`GosubDocumentAdapter<C: HasDocument>`** implements that trait over an
`Arc<C::Document>` from world 1. It is where all the translation lives:

- **Lazy computed styles**: on first `get_own_style` for a node, the adapter runs world 1's
  CSS selector matching (`C::CssSystem`) and caches the resulting property map per node.
  Nothing is computed for nodes the pipeline never asks about.
- **Value translation**: `css_property_to_value` maps each generic `CssProperty` into the
  pipeline's closed `Value` enum (colors, display keywords, lengths, gradients, …).
- **Inline styles**: the `style=""` attribute is parsed and cached separately, taking
  precedence as highest-specificity.
- **Generated content**: `::before` / `::after` have no DOM node, so the adapter mints
  *synthetic* `NodeId`s (bit-encoded: flag + role + owner id) and materializes pseudo-boxes
  lazily. The rest of the pipeline treats them as ordinary nodes.
- **Invalidation**: `invalidate_style_for_nodes` / `clear_style_cache` let hover repaints
  re-run selector matching (`:hover`) for just the affected nodes.

The handoff happens in `gosub_engine`'s pipeline entry points
(`crates/gosub_engine/src/engine/context.rs`): each rebuild wraps the parsed document in a
fresh adapter and hands it to the pipeline's render-tree builder:

```rust
let adapter = GosubDocumentAdapter::<C>::new(doc);   // world 1 in, world 2 out
let mut render_tree = RenderTree::new(Arc::new(adapter));
```

Everything upstream of that line is world 1; everything downstream is world 2.

## The full picture

```text
        WORLD 1 (gosub_interface traits)          │            WORLD 2 (pipeline types)
                                                  │
  HTML  ──► gosub_html5 ──► C::Document ──┐       │
                                          ├──► GosubDocumentAdapter ──► RenderTree ──► layout
  CSS   ──► gosub_css3  ──► stylesheets ──┘       │   (PipelineDocument)              ──► … ──► pixels
                                                  │
  gosub_taffy (Layouter impl, dormant)            │   layouter/taffy.rs (the live layouter)
```

## Duplications to be aware of

| Concept | World 1 | World 2 |
|---|---|---|
| Layouter struct | `gosub_taffy::TaffyLayouter` (dormant) | `gosub_render_pipeline::layouter::taffy::TaffyLayouter` (live) |
| Layouter trait | `gosub_interface::layout::Layouter` | pipeline-local `CanLayout` |
| Lattice table bridge | `gosub_taffy`'s `TableTree` adapter | `PipelineTableTree` in `layouter/table.rs` |
| Style representation | `CssSystem` property maps (general) | `StyleProperty`/`Value` enums (closed, render-oriented) |
| Node identity | `gosub_shared::node::NodeId` | same `NodeId`, plus synthetic pseudo-element ids |

When you search the workspace for `TaffyLayouter`, check the crate before drawing
conclusions — the two share a name and a dependency, nothing else.

## Why it is this way (and where it might go)

The trade is isolation versus duplication. Owning its document model lets the pipeline be
developed and tested without routing every experiment through the full parse/cascade
machinery, and gives the painter a closed style enum it can match on exhaustively; the
cost is the duplicated layouters/bridges above and a translation layer on every rebuild.
Should the two layouters ever merge, `gosub_taffy`'s trait-over-the-real-DOM approach and
the pipeline's battle-tested inline/table handling are the two halves worth keeping.
