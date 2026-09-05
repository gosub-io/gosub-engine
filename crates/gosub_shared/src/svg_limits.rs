//! Depth limit for untrusted SVG.
//!
//! Parsing SVG costs a stack frame per level of element nesting: roxmltree's tokenizer recurses
//! between `parse_element` and `parse_content`, and nothing below us bounds that. Its
//! `nodes_limit` counts nodes rather than depth, and usvg's own 1024-level guard sits in a later
//! pass the tokenizer never reaches. A few kilobytes of `<g><g><g>...` will therefore abort the
//! process on a stack overflow (GHSA-c762-mxfh-vwvp).
//!
//! [`xml_nesting_depth_exceeds`] rejects those documents before the parser sees them.
//! [`SVG_PARSE_STACK_SIZE`] is the other half: the depth limit only bounds the number of frames,
//! and callers cannot say how much of their own stack is already spent.

/// Maximum element nesting depth accepted in an SVG document.
///
/// The 5887 SVG files in this machine's icon themes nest at most 8 levels deep, so this is not a
/// limit real content runs into. Unoptimised, the tokenizer spends roughly 15 KiB of stack per
/// level, which puts 128 levels at ~2 MiB - a quarter of [`SVG_PARSE_STACK_SIZE`].
pub const MAX_SVG_NESTING_DEPTH: usize = 128;

/// Stack size for the thread an SVG parse runs on.
pub const SVG_PARSE_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Levels of general-entity expansion roxmltree performs (`LoopDetector::inc_depth`).
///
/// Entity replacement text is re-parsed as markup, so `<!ENTITY e "<g><g>...">` contributes its
/// nesting wherever `&e;` appears, and entities referencing entities multiply that. The scan below
/// sees each declaration once, so a document carrying an internal subset gets its budget divided
/// by this to stay an upper bound.
const ENTITY_NESTING_LIMIT: usize = 10;

/// Whether `xml` nests elements deeper than `max`.
///
/// Answers the depth question only, and answers it high: anything the scan cannot account for is
/// counted as nesting rather than skipped, so a document it accepts really is within `max`.
/// Malformed input may be rejected, which the parser would have done anyway.
pub fn xml_nesting_depth_exceeds(xml: &[u8], max: usize) -> bool {
    let limit = if has_internal_dtd_subset(xml) {
        max / ENTITY_NESTING_LIMIT
    } else {
        max
    };

    let mut depth: usize = 0;
    let mut i = 0;

    while i < xml.len() {
        if xml[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &xml[i..];

        // Not markup to the parser either. An unterminated one is a parse error, so giving up on
        // the rest of the document can only reject something already doomed.
        if let Some(end) = skip_delimited(rest, b"<!--", b"-->")
            .or_else(|| skip_delimited(rest, b"<![CDATA[", b"]]>"))
            .or_else(|| skip_delimited(rest, b"<?", b"?>"))
        {
            i += end;
            continue;
        }

        if rest.starts_with(b"</") {
            // Saturating, so a stray close tag cannot wrap the counter round to a huge depth.
            depth = depth.saturating_sub(1);
            i += tag_end(rest, 2).0;
            continue;
        }

        // Declarations are not skipped: markup inside `<!ENTITY name "...">` is markup the parser
        // will expand, so counting it where it is declared is what keeps this an upper bound.

        if !is_name_start(rest.get(1).copied()) {
            // A bare `<` in text - not well-formed, but treating it as text means a real element
            // can never hide behind one.
            i += 1;
            continue;
        }

        let (len, self_closing) = tag_end(rest, 1);
        if !self_closing {
            depth += 1;
            if depth > limit {
                return true;
            }
        }
        i += len;
    }

    false
}

/// Whether the DOCTYPE carries an internal subset (`<!DOCTYPE svg [ ... ]>`), the only place a
/// document can declare its own entities.
fn has_internal_dtd_subset(xml: &[u8]) -> bool {
    let Some(start) = find(xml, b"<!DOCTYPE") else {
        return false;
    };
    // The subset opens before the DOCTYPE's own `>`.
    xml[start..].iter().take_while(|&&b| b != b'>').any(|&b| b == b'[')
}

/// If `xml` opens with `open`, the length through the matching `close`; the whole remaining length
/// if `close` never arrives.
fn skip_delimited(xml: &[u8], open: &[u8], close: &[u8]) -> Option<usize> {
    if !xml.starts_with(open) {
        return None;
    }
    Some(match find(&xml[open.len()..], close) {
        Some(at) => open.len() + at + close.len(),
        None => xml.len(),
    })
}

/// Length of the tag starting at `xml[0]`, scanning from `from`, and whether it self-closed.
///
/// Quoted attribute values are opaque, so a `>` or `/` inside one neither ends the tag nor makes
/// it look self-closing.
fn tag_end(xml: &[u8], from: usize) -> (usize, bool) {
    let mut quote: Option<u8> = None;
    let mut last = 0u8;

    for (offset, &b) in xml.iter().enumerate().skip(from) {
        if let Some(q) = quote {
            if b == q {
                quote = None;
                last = q;
            }
            continue;
        }
        match b {
            b'"' | b'\'' => quote = Some(b),
            b'>' => return (offset + 1, last == b'/'),
            _ if b.is_ascii_whitespace() => {}
            _ => last = b,
        }
    }

    (xml.len(), false)
}

/// Loose on purpose. The exact XML NameStartChar production does not matter here, only that a `<`
/// the parser would read as an element start is never dismissed as text.
fn is_name_start(b: Option<u8>) -> bool {
    matches!(b, Some(b) if b.is_ascii_alphabetic() || b == b'_' || b == b':' || !b.is_ascii())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `depth` levels of `<g>` around a leaf.
    fn nested(depth: usize) -> Vec<u8> {
        let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg">"#);
        s.push_str(&"<g>".repeat(depth));
        s.push_str("<rect/>");
        s.push_str(&"</g>".repeat(depth));
        s.push_str("</svg>");
        s.into_bytes()
    }

    #[test]
    fn depth_counts_from_the_root() {
        // <svg> plus 8 <g> is 9 levels; the <rect/> self-closes and adds none.
        assert!(!xml_nesting_depth_exceeds(&nested(8), 9));
        assert!(xml_nesting_depth_exceeds(&nested(8), 8));
    }

    #[test]
    fn default_limit_accepts_real_svg_and_rejects_the_poc() {
        assert!(!xml_nesting_depth_exceeds(&nested(8), MAX_SVG_NESTING_DEPTH));
        // 136 was the shallowest document that overflowed a 2 MiB stack unoptimised; the proof of
        // concept nests 2000.
        assert!(xml_nesting_depth_exceeds(&nested(136), MAX_SVG_NESTING_DEPTH));
        assert!(xml_nesting_depth_exceeds(&nested(2000), MAX_SVG_NESTING_DEPTH));
    }

    #[test]
    fn self_closing_tags_do_not_nest() {
        let doc = format!("<svg>{}</svg>", "<rect/>".repeat(500));
        assert!(!xml_nesting_depth_exceeds(doc.as_bytes(), 4));

        // A trailing `/` inside a quoted value is not a self-close: <svg><a> is 2 levels.
        let doc = br#"<svg><a href="x/"><g/></a></svg>"#;
        assert!(!xml_nesting_depth_exceeds(doc, 2));
        assert!(xml_nesting_depth_exceeds(doc, 1));
    }

    #[test]
    fn markup_in_attribute_values_is_not_nesting() {
        let doc = br#"<svg><desc title="a &gt; b <g><g><g>"><rect/></desc></svg>"#;
        assert!(!xml_nesting_depth_exceeds(doc, 3));
    }

    #[test]
    fn comments_cdata_and_pis_are_skipped() {
        let inner = "<g>".repeat(500);
        for doc in [
            format!("<?xml version=\"1.0\"?><svg><!-- {inner} --><rect/></svg>"),
            format!("<svg><![CDATA[{inner}]]><rect/></svg>"),
        ] {
            assert!(!xml_nesting_depth_exceeds(doc.as_bytes(), 4), "{doc:.60}");
        }
    }

    #[test]
    fn unterminated_comment_ends_the_scan() {
        assert!(!xml_nesting_depth_exceeds(b"<svg><!-- <g><g><g>", 1));
    }

    #[test]
    fn stray_less_than_is_text() {
        assert!(!xml_nesting_depth_exceeds(b"<svg>a < b</svg>", 1));
    }

    #[test]
    fn stray_close_tags_do_not_underflow() {
        assert!(!xml_nesting_depth_exceeds(b"</g></g><svg><g/></svg>", 1));
    }

    #[test]
    fn doctype_without_a_subset_keeps_the_full_budget() {
        let mut doc = br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "svg11.dtd">"#.to_vec();
        doc.extend_from_slice(&nested(20));
        assert!(!xml_nesting_depth_exceeds(&doc, MAX_SVG_NESTING_DEPTH));
    }

    #[test]
    fn entity_declarations_are_counted() {
        // Expands to a document thousands of levels deep although nothing at the top level nests.
        // The `<g>`s sit in a quoted value, which an element tag would have treated as opaque.
        let bomb = format!(
            "<!DOCTYPE svg [<!ENTITY deep \"{}{}\">]><svg>&deep;</svg>",
            "<g>".repeat(5000),
            "</g>".repeat(5000),
        );
        assert!(xml_nesting_depth_exceeds(bomb.as_bytes(), MAX_SVG_NESTING_DEPTH));
    }

    #[test]
    fn an_internal_subset_shrinks_the_budget() {
        let shallow = format!("<!DOCTYPE svg [<!ENTITY e \"x\">]>{}", ascii(&nested(8)));
        assert!(!xml_nesting_depth_exceeds(shallow.as_bytes(), MAX_SVG_NESTING_DEPTH));

        // 20 levels is fine on its own, but not against a tenth of the budget.
        let deeper = format!("<!DOCTYPE svg [<!ENTITY e \"x\">]>{}", ascii(&nested(20)));
        assert!(xml_nesting_depth_exceeds(deeper.as_bytes(), MAX_SVG_NESTING_DEPTH));
    }

    fn ascii(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
