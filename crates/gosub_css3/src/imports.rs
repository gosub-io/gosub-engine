//! `@import` resolution: pulling imported stylesheets in and splicing their rules into place.
//!
//! This crate has neither a network stack nor a URL resolver, so the host supplies both
//! through a callback and everything CSS-shaped stays here: cascade order, the media
//! conditions an import imposes on what it brings in, `supports()` gating, cycle detection
//! and the recursion budget.

use std::collections::HashSet;
use std::sync::Arc;

use crate::stylesheet::CssStylesheet;
use crate::Css3;
use gosub_shared::config::ParserConfig;

/// How deep a chain of imports may go. Chains beyond two or three are already unusual;
/// this only exists so a hostile stylesheet cannot drive unbounded recursion.
const MAX_IMPORT_DEPTH: usize = 8;

/// Ceiling on how many stylesheets one root may pull in. Cycle detection is per-path (so a
/// sheet legitimately imported down two different branches is fetched for both, as the
/// cascade requires), which leaves a diamond able to fan out; this bounds it.
const MAX_IMPORTED_SHEETS: usize = 64;

/// Fetches an imported stylesheet.
///
/// Called with `(base_url, requested_url)` - the importing sheet's own URL and the target
/// exactly as written - and returns the absolute URL actually loaded together with its text.
/// `None` for anything that could not be fetched, which drops that import and no more.
pub type ImportFetcher<'a> = dyn FnMut(&str, &str) -> Option<(String, String)> + 'a;

/// Resolve every `@import` in `sheet`, depth-first, splicing what they bring in ahead of the
/// sheet's own rules.
///
/// Imported rules must cascade *below* the importing sheet's, and `@import` is only valid
/// before any style rule, so "in front, in import order" is the whole of the ordering rule.
pub fn resolve_imports(sheet: &mut CssStylesheet, fetch: &mut ImportFetcher<'_>) {
    if sheet.imports.is_empty() {
        return;
    }
    // The root is on the path from the start, so a sheet importing itself is caught.
    let mut path = HashSet::from([sheet.url.clone()]);
    let mut budget = MAX_IMPORTED_SHEETS;
    resolve_into(sheet, fetch, &mut path, &mut budget, 0);
}

fn resolve_into(
    sheet: &mut CssStylesheet,
    fetch: &mut ImportFetcher<'_>,
    path: &mut HashSet<String>,
    budget: &mut usize,
    depth: usize,
) {
    // Take the list: an import is resolved once, and the record is spent doing it.
    let imports = std::mem::take(&mut sheet.imports);
    if depth >= MAX_IMPORT_DEPTH {
        log::warn!("@import chain deeper than {MAX_IMPORT_DEPTH} levels, ignoring the rest");
        return;
    }

    let base = sheet.url.clone();
    let origin = sheet.origin;
    let mut insert_at = 0;

    for import in imports {
        // A condition that does not hold means the sheet is never even requested.
        if let Some(condition) = &import.supports {
            if !condition.matches() {
                continue;
            }
        }

        if *budget == 0 {
            log::warn!("@import budget of {MAX_IMPORTED_SHEETS} stylesheets exhausted");
            break;
        }

        let Some((url, text)) = fetch(&base, &import.url) else {
            continue;
        };

        // Cycle detection is per-path: the sheet is on the path only while its own subtree is
        // being resolved, so a shared reset imported by two siblings still lands in both.
        if !path.insert(url.clone()) {
            log::warn!("Skipping circular @import of {url}");
            continue;
        }
        *budget -= 1;

        let config = ParserConfig {
            source: Some(url.clone()),
            ignore_errors: true,
            ..Default::default()
        };
        // An imported sheet carries the importing sheet's origin: it is part of the same
        // author (or user, or UA) stylesheet as far as the cascade is concerned.
        match Css3::parse_str(&text, config, origin, &url) {
            Ok(mut imported) => {
                // Depth-first, so a nested import's rules end up inside this import's block
                // rather than after it.
                resolve_into(&mut imported, fetch, path, budget, depth + 1);
                let media = import.media.map(Arc::new);
                insert_at = sheet.splice_import(imported, media.as_ref(), insert_at);
            }
            Err(err) => log::warn!("Could not parse imported stylesheet {url}: {err}"),
        }

        path.remove(&url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_query::MediaEnvironment;
    use crate::stylesheet::CssSelectorPart;
    use gosub_interface::css3::CssOrigin;
    use std::collections::HashMap;

    /// Build a fetcher over a fixed set of stylesheets, and record what it was asked for.
    fn fetcher(sheets: HashMap<&'static str, &'static str>) -> impl FnMut(&str, &str) -> Option<(String, String)> {
        move |_base, requested| {
            sheets
                .get(requested)
                .map(|text| (requested.to_string(), (*text).to_string()))
        }
    }

    fn parse(css: &str) -> CssStylesheet {
        Css3::parse_str(
            css,
            ParserConfig {
                ignore_errors: true,
                ..Default::default()
            },
            CssOrigin::Author,
            "root.css",
        )
        .expect("root stylesheet should parse")
    }

    fn selectors(sheet: &CssStylesheet) -> Vec<String> {
        sheet
            .rules
            .iter()
            .map(|rule| match &rule.selectors[0].parts[0][0] {
                CssSelectorPart::Type(name) => name.clone(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    /// Imported rules cascade below the importing sheet's own, so they go in front.
    #[test]
    fn imported_rules_are_spliced_ahead_in_import_order() {
        let mut sheet = parse(
            r#"
            @import "a.css";
            @import "b.css";
            h3 { color: green; }
            "#,
        );
        let mut fetch = fetcher(HashMap::from([
            ("a.css", "h1 { color: red; }"),
            ("b.css", "h2 { color: blue; }"),
        ]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(selectors(&sheet), vec!["h1", "h2", "h3"]);
        assert!(sheet.imports.is_empty(), "resolved imports are consumed");
    }

    /// A nested import lands inside its parent's block, not after it.
    #[test]
    fn nested_imports_resolve_depth_first() {
        let mut sheet = parse(
            r#"
            @import "outer.css";
            own { color: green; }
            "#,
        );
        let mut fetch = fetcher(HashMap::from([
            ("outer.css", "@import \"inner.css\"; outer { color: red; }"),
            ("inner.css", "inner { color: blue; }"),
        ]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(selectors(&sheet), vec!["inner", "outer", "own"]);
    }

    /// The import's media query travels with every rule it brings in.
    #[test]
    fn import_media_gates_the_imported_rules() {
        let mut sheet = parse(r#"@import "print.css" print;"#);
        let mut fetch = fetcher(HashMap::from([("print.css", "h1 { color: red; }")]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(sheet.rules.len(), 1);
        let screen = MediaEnvironment::default();
        assert!(
            !sheet.rules[0].media_matches(&screen),
            "a print-only import must not apply on screen"
        );
    }

    /// An import's media condition combines with one already inside the imported sheet.
    #[test]
    fn import_media_stacks_with_inner_media() {
        let mut sheet = parse(r#"@import "wide.css" screen;"#);
        let mut fetch = fetcher(HashMap::from([(
            "wide.css",
            "@media (min-width: 600px) { h1 { color: red; } }",
        )]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(sheet.rules.len(), 1);
        let media = sheet.rules[0].media.as_ref().expect("both conditions recorded");
        assert_eq!(media.len(), 2, "the import's condition and the inner one");
        assert!(sheet.rules[0].media_matches(&MediaEnvironment {
            width: 800.0,
            ..Default::default()
        }));
        assert!(!sheet.rules[0].media_matches(&MediaEnvironment {
            width: 400.0,
            ..Default::default()
        }));
    }

    /// A guard the engine fails means the sheet is never requested at all.
    #[test]
    fn unsupported_condition_skips_the_fetch() {
        let mut sheet = parse(r#"@import "grid.css" supports(display: bogus-value);"#);
        let mut asked = Vec::new();
        {
            let mut fetch = |_base: &str, requested: &str| {
                asked.push(requested.to_string());
                Some((requested.to_string(), "h1 { color: red; }".to_string()))
            };
            resolve_imports(&mut sheet, &mut fetch);
        }
        assert!(asked.is_empty(), "the guarded sheet must not be fetched");
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn circular_imports_terminate() {
        let mut sheet = parse(r#"@import "a.css";"#);
        let mut fetch = fetcher(HashMap::from([
            ("a.css", "@import \"b.css\"; a { color: red; }"),
            ("b.css", "@import \"a.css\"; b { color: blue; }"),
        ]));
        resolve_imports(&mut sheet, &mut fetch);

        // Both sheets contribute once; the loop back to a.css is refused.
        assert_eq!(selectors(&sheet), vec!["b", "a"]);
    }

    #[test]
    fn a_sheet_that_imports_itself_terminates() {
        let mut sheet = parse(r#"@import "root.css";"#);
        let mut fetch = fetcher(HashMap::from([("root.css", "@import \"root.css\"; a { color: red; }")]));
        resolve_imports(&mut sheet, &mut fetch);
        assert!(sheet.rules.is_empty(), "the root is already on the path");
    }

    /// The same sheet reached down two branches is legitimately included twice.
    #[test]
    fn diamond_imports_are_not_deduplicated() {
        let mut sheet = parse(
            r#"
            @import "left.css";
            @import "right.css";
            "#,
        );
        let mut fetch = fetcher(HashMap::from([
            ("left.css", "@import \"shared.css\"; left { color: red; }"),
            ("right.css", "@import \"shared.css\"; right { color: blue; }"),
            ("shared.css", "shared { color: green; }"),
        ]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(selectors(&sheet), vec!["shared", "left", "shared", "right"]);
    }

    /// A fetch that fails drops only its own import.
    #[test]
    fn a_failed_fetch_does_not_lose_the_other_rules() {
        let mut sheet = parse(
            r#"
            @import "missing.css";
            @import "present.css";
            own { color: green; }
            "#,
        );
        let mut fetch = fetcher(HashMap::from([("present.css", "present { color: red; }")]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(selectors(&sheet), vec!["present", "own"]);
    }

    /// A long chain stops at the depth limit rather than recursing without bound.
    #[test]
    fn depth_limit_is_enforced() {
        let mut sheet = parse(r#"@import "deep.css";"#);
        // Every level imports itself under a fresh name, so the chain never ends on its own.
        let mut level = 0;
        let mut fetch = |_base: &str, _requested: &str| {
            level += 1;
            Some((
                format!("deep{level}.css"),
                format!("@import \"deep.css\"; s{level} {{ color: red; }}"),
            ))
        };
        resolve_imports(&mut sheet, &mut fetch);

        assert!(
            sheet.rules.len() <= MAX_IMPORT_DEPTH,
            "expected at most {MAX_IMPORT_DEPTH} levels, got {}",
            sheet.rules.len()
        );
        assert!(!sheet.rules.is_empty(), "the chain should still contribute what it can");
    }

    /// Font faces from an imported sheet come along.
    #[test]
    fn imported_font_faces_are_kept() {
        let mut sheet = parse(r#"@import "fonts.css";"#);
        let mut fetch = fetcher(HashMap::from([(
            "fonts.css",
            "@font-face { font-family: 'X'; src: url(x.ttf); }",
        )]));
        resolve_imports(&mut sheet, &mut fetch);

        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.font_faces[0].family, "X");
    }
}
