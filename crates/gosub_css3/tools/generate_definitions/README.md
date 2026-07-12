# generate_definitions

Generates the CSS property/value definition files used by the `gosub_css3`
crate to validate CSS declarations.

The generated JSON files live in `crates/gosub_css3/resources/definitions/`
and are embedded into the crate at compile time (see
`src/matcher/property_definitions.rs`). They describe, for every CSS property:
its value grammar (in [CSS value definition syntax](https://developer.mozilla.org/en-US/docs/Web/CSS/Value_definition_syntax)),
its initial value, and whether it is inherited — plus the shared value types
(`<length>`, `<color>`, …), at-rules, and selectors those grammars reference.
The CSS style matcher uses this data as its validation gate when resolving
declarations.

## Data sources

The tool merges two upstream datasets, downloaded at run time:

- **[w3c/webref](https://github.com/w3c/webref)** (`ed/css/*.json` on the
  `curated` branch) — machine-extracted definitions from the W3C editor's
  draft specs: property grammars, value types, at-rules, and selectors.
  Versioned per-level snapshots (`css-backgrounds-4.json`, …) are skipped in
  favor of the unversioned extract.
- **[mdn/data](https://github.com/mdn/data)** (`css/properties.json` and
  `css/syntaxes.json`) — MDN's property dataset and value-type dictionary.

MDN is the authoritative property *set*: it tracks the full shipping surface,
including vendor-prefixed and legacy properties that the standards-scoped
webref omits. For each MDN property the tool prefers webref's spec grammar,
falling back to MDN's own syntax when webref has no entry. Two backfill passes
then fill grammar gaps: MDN value types that webref does not define, and
webref sub-properties (e.g. `<'box-shadow-blur'>`) that other grammars
reference as value types.

Webref files are cached in a local `.css_cache/` directory (git-ignored,
created next to wherever you run the tool). Cache entries are validated
against the upstream git blob SHA, so a re-run only downloads files that
changed upstream.

## Usage

From this directory (the output paths are relative to the working directory):

```sh
cargo run -p generate_definitions
```

The output is written to `.output/definitions/`:

- `definitions.json` — everything in a single file
- `definitions_properties.json`, `definitions_values.json`,
  `definitions_at-rules.json`, `definitions_selectors.json` — the same data
  split per category (the properties and values files are what the crate
  embeds)

Output is fully deterministic — spec files are merged in a fixed order and
every collection is sorted — so regeneration produces minimal diffs.

To update the definitions the engine actually uses, copy the generated files
over the checked-in ones and review the diff:

```sh
cp .output/definitions/definitions*.json ../../resources/definitions/
```

Then run the `gosub_css3` tests — the property definitions are exercised by
the matcher tests:

```sh
cargo test -p gosub_css3
```

## History

This tool is a Rust port of an earlier Go implementation that lived in this
directory (removed in the same change that added this crate; see git history).
It ports the generator as reworked on the `css3-rewrite` branch — MDN as the
authoritative property set, webref grammar preferred, comma-list-idiom
normalization, and the two backfill passes — not the older webref-authoritative
merge that was on `main`. Against that css3-rewrite Go version the output is
byte-identical, with one deliberate exception:
where two specs define the same value type with different grammars, the Go
tool's winner depended on download-goroutine completion order, while this port
always merges spec files in listing order, making conflicts (e.g.
`<content-list>`, defined by both css-content and css-gcpm) resolve
deterministically. The Go tool's unused patching machinery (local `.patch`
files applied to cached webref data) was dropped in the port.
