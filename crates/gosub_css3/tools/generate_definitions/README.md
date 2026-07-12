# generate_definitions

A Go tool that generates the CSS property/value definition files used by the
`gosub_css3` crate to validate CSS declarations.

The generated JSON files live in `crates/gosub_css3/resources/definitions/` and
are embedded into the crate at compile time (see
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
- **[mdn/data](https://github.com/mdn/data)** (`css/properties.json`) — used to
  enrich webref properties with their `computed` and `initial` metadata.

Webref files are cached in a local `.css_cache/` directory (git-ignored,
created next to wherever you run the tool). Cache entries are validated
against the upstream git blob SHA, so a re-run only downloads files that
changed upstream. The MDN dataset is currently fetched on every run.

Duplicate definitions across spec files are detected and logged; when two
specs disagree on a grammar the tool logs both and keeps the latest one.

## Usage

Requires a Go toolchain (no Rust involved). From this directory:

```sh
go run .
```

The output is written to `.output/definitions/`:

- `definitions.json` — everything in a single file
- `definitions_properties.json`, `definitions_values.json`,
  `definitions_at-rules.json`, `definitions_selectors.json` — the same data
  split per category (this is what the crate embeds)

Which of the two forms is emitted is controlled by the `exportType` constant
in `main.go` (default: both). Output is sorted by name so regeneration
produces minimal diffs.

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

## Patching upstream data

Sometimes the upstream data is wrong or incompatible with our matcher. The
`patch` package can apply `.patch` files to the cached webref JSON before it
is parsed, and keeps an index (`.css_cache/index/cache_index.json`) of which
patches have been applied to which files so they are not applied twice.
Custom patches go in `.css_cache/patches/`. Note that patch application is
not currently invoked from `main.go` (`DownloadPatches` is commented out);
the machinery is kept for when spec data needs local fixes again.

## Package layout

- `main.go` — orchestrates the merge and writes the output JSON
- `webref/` — downloads, caches, and parses the webref CSS spec files;
  normalizes grammars (e.g. quoting literal parentheses) and merges
  duplicates across specs
- `mdn/` — fetches MDN's `properties.json`
- `patch/` — patch application and cache-index bookkeeping (currently unused)
- `specs/` — fetches the webref spec index (currently unused; previously used
  to filter to W3C specs only)
- `utils/` — shared types, GitHub blob SHA hashing, cache constants
