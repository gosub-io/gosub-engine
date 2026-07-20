# gosub_svg

SVG document support for the Gosub browser engine. This crate parses SVG — either from a
string or from an `<svg>` subtree of an HTML document — into a `usvg::Tree`. It does not
rasterize anything itself; render backends consume the tree and draw it.

## Entry points

- `SVGDocument::from_str(svg)` — parse SVG text.
- `SVGDocument::from_html_doc::<C>(node_id, doc)` — serialize an HTML DOM subtree back to
  markup and parse it as SVG; this is the bridge from `gosub_html5` documents.

The public surface is intentionally one struct: `SVGDocument { pub tree: usvg::Tree }`.
System fonts are loaded once into a process-wide `usvg` font database on first use.

## Further reading

- [docs/crates.md](../../docs/crates.md) — where this crate sits in the workspace
- [docs/render-pipeline/](../../docs/render-pipeline/) — how parsed media reaches the
  backends that rasterize it
