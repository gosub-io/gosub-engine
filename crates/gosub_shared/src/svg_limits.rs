//! Depth limit for untrusted SVG.
//!
//! Parsing SVG costs a stack frame per level of element nesting: roxmltree's tokenizer recurses
//! between `parse_element` and `parse_content`, and nothing below us bounds that. Its
//! `nodes_limit` counts nodes rather than depth, and usvg's own 1024-level guard sits in a later
//! pass the tokenizer never reaches. A few kilobytes of `<g><g><g>...` will therefore abort the
//! process on a stack overflow (GHSA-c762-mxfh-vwvp).
//!
//! [`xml_exceeds_limits`] rejects those documents before the parser sees them.
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

/// Why a document must not be handed to the XML parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmlLimit {
    /// Nests elements deeper than the limit.
    Depth,
    /// Declares an entity whose replacement text contains markup. That text is re-parsed as
    /// markup, so `<!ENTITY e "<g><g>...">` contributes nesting wherever `&e;` appears, and
    /// entities referencing entities multiply it up to ten times over (roxmltree's
    /// `LoopDetector::inc_depth`). Bounding that from outside means deciding what the parser
    /// will do with a DTD, which is its job and not reliably ours, so these are refused.
    ///
    /// Entities holding no markup are left alone: they cannot expand into elements, so they
    /// cannot add depth. That distinction is not academic - Illustrator's SVG export declares
    /// entities for style strings (`<!ENTITY st0 "fill-rule:nonzero;...">`), and one such icon
    /// turned up in the 21265-file corpus this was checked against.
    Entities,
}

/// Whether `xml` must be kept away from the parser, and why.
///
/// A scanner, not a validator: it answers high. Anything it cannot account for counts as nesting
/// rather than being skipped, so a document it accepts really is within `max`. Malformed input
/// may be rejected, which the parser would have done anyway.
pub fn xml_exceeds_limits(xml: &[u8], max: usize) -> Option<XmlLimit> {
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

        // Reached only outside comments, CDATA and processing instructions, and outside element
        // tags - those are skipped whole below, quoted attribute values included - so this is a
        // real declaration rather than the text `<!ENTITY` appearing somewhere harmless.
        // Matched case-insensitively: XML requires upper case here, and over-matching can only
        // reject a document the parser would also refuse.
        if starts_with_ignore_ascii_case(rest, b"<!ENTITY") {
            match entity_literal(rest) {
                // No markup in the replacement text, so it cannot expand into elements.
                Some((false, len)) => {
                    i += len;
                    continue;
                }
                // Markup, or a declaration this scan cannot read - refuse either way.
                _ => return Some(XmlLimit::Entities),
            }
        }

        if rest.starts_with(b"</") {
            // Saturating, so a stray close tag cannot wrap the counter round to a huge depth.
            depth = depth.saturating_sub(1);
            i += tag_end(rest, 2).0;
            continue;
        }

        // Other declarations are not skipped. Skipping `<!DOCTYPE ...>` would mean finding where
        // it ends, and its system identifier is a quoted string that may contain `>` - the kind
        // of parser detail this scan deliberately does not try to know. Scanning through it is
        // harmless: it holds no element tags.

        if !is_name_start(rest.get(1).copied()) {
            // A bare `<` in text - not well-formed, but treating it as text means a real element
            // can never hide behind one.
            i += 1;
            continue;
        }

        let (len, self_closing) = tag_end(rest, 1);
        if !self_closing {
            depth += 1;
            if depth > max {
                return Some(XmlLimit::Depth);
            }
        }
        i += len;
    }

    None
}

/// The quoted replacement text of the `<!ENTITY ...>` declaration starting at `xml[0]`: whether
/// it contains markup, and the length through the closing quote.
///
/// `None` when no literal can be found, which includes a malformed or unterminated declaration.
/// Callers refuse in that case rather than guess. An external entity (`<!ENTITY e SYSTEM "x.dtd">`)
/// yields its system identifier, which holds no markup and is never fetched by roxmltree anyway.
fn entity_literal(xml: &[u8]) -> Option<(bool, usize)> {
    let mut i = "<!ENTITY".len();
    while i < xml.len() && xml[i] != b'"' && xml[i] != b'\'' {
        // The declaration ended before any literal appeared.
        if xml[i] == b'>' {
            return None;
        }
        i += 1;
    }
    let quote = *xml.get(i)?;
    i += 1;
    let start = i;
    while i < xml.len() && xml[i] != quote {
        i += 1;
    }
    if i >= xml.len() {
        return None;
    }
    // A character reference such as `&#60;` is text once expanded, not a tag, so a literal `<`
    // is the only thing that can open an element.
    Some((xml[start..i].contains(&b'<'), i + 1))
}

/// ASCII-case-insensitive `starts_with`.
fn starts_with_ignore_ascii_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack[..needle.len()].eq_ignore_ascii_case(needle)
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

    fn check(xml: &[u8], max: usize) -> Option<XmlLimit> {
        xml_exceeds_limits(xml, max)
    }

    #[test]
    fn depth_counts_from_the_root() {
        // <svg> plus 8 <g> is 9 levels; the <rect/> self-closes and adds none.
        assert_eq!(check(&nested(8), 9), None);
        assert_eq!(check(&nested(8), 8), Some(XmlLimit::Depth));
    }

    #[test]
    fn default_limit_accepts_real_svg_and_rejects_the_poc() {
        assert_eq!(check(&nested(8), MAX_SVG_NESTING_DEPTH), None);
        // 136 was the shallowest document that overflowed a 2 MiB stack unoptimised; the proof of
        // concept nests 2000.
        assert_eq!(check(&nested(136), MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Depth));
        assert_eq!(check(&nested(2000), MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Depth));
    }

    #[test]
    fn self_closing_tags_do_not_nest() {
        let doc = format!("<svg>{}</svg>", "<rect/>".repeat(500));
        assert_eq!(check(doc.as_bytes(), 4), None);

        // A trailing `/` inside a quoted value is not a self-close: <svg><a> is 2 levels.
        let doc = br#"<svg><a href="x/"><g/></a></svg>"#;
        assert_eq!(check(doc, 2), None);
        assert_eq!(check(doc, 1), Some(XmlLimit::Depth));
    }

    #[test]
    fn markup_in_attribute_values_is_not_nesting() {
        let doc = br#"<svg><desc title="a &gt; b <g><g><g>"><rect/></desc></svg>"#;
        assert_eq!(check(doc, 3), None);
    }

    #[test]
    fn comments_cdata_and_pis_are_skipped() {
        let inner = "<g>".repeat(500);
        for doc in [
            format!("<?xml version=\"1.0\"?><svg><!-- {inner} --><rect/></svg>"),
            format!("<svg><![CDATA[{inner}]]><rect/></svg>"),
        ] {
            assert_eq!(check(doc.as_bytes(), 4), None, "{doc:.60}");
        }
    }

    #[test]
    fn unterminated_comment_ends_the_scan() {
        assert_eq!(check(b"<svg><!-- <g><g><g>", 1), None);
    }

    #[test]
    fn stray_less_than_is_text() {
        assert_eq!(check(b"<svg>a < b</svg>", 1), None);
    }

    #[test]
    fn stray_close_tags_do_not_underflow() {
        assert_eq!(check(b"</g></g><svg><g/></svg>", 1), None);
    }

    #[test]
    fn a_doctype_without_entities_is_fine() {
        // External DTDs are what real SVG carries, and roxmltree does not fetch them.
        let mut doc = br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "svg11.dtd">"#.to_vec();
        doc.extend_from_slice(&nested(20));
        assert_eq!(check(&doc, MAX_SVG_NESTING_DEPTH), None);
    }

    #[test]
    fn entities_holding_markup_are_refused() {
        let doc = br#"<!DOCTYPE svg [<!ENTITY e "<g><g/></g>">]><svg>&e;</svg>"#;
        assert_eq!(check(doc, MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Entities));
    }

    #[test]
    fn entities_holding_no_markup_are_allowed() {
        // What Illustrator's SVG export emits: style strings, referenced as `style="&st0;"`.
        // These cannot expand into elements, so they cannot add depth.
        let doc = br#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG//EN" "svg.dtd" [
            <!ENTITY st0 "fill-rule:nonzero;stroke:#FFFFFF;stroke-width:6.6871;">
            <!ENTITY st1 "fill-rule:evenodd;clip-rule:evenodd;">
        ]><svg><path style="&st0;"/></svg>"#;
        assert_eq!(check(doc, MAX_SVG_NESTING_DEPTH), None);
    }

    #[test]
    fn a_malformed_entity_declaration_is_refused() {
        // No readable literal, so the scan cannot tell what it expands to.
        assert_eq!(
            check(br#"<!DOCTYPE svg [<!ENTITY e>]><svg/>"#, 4),
            Some(XmlLimit::Entities)
        );
        assert_eq!(
            check(br#"<!DOCTYPE svg [<!ENTITY e "unterminated"#, 4),
            Some(XmlLimit::Entities)
        );
    }

    /// The internal subset used to be located by scanning from `<!DOCTYPE` to the first `>`, which
    /// a `>` inside the quoted system identifier cut short - so the subset went unseen and the
    /// entity bomb behind it got the full depth budget. Found by CodeRabbit on PR #1229.
    #[test]
    fn a_quoted_gt_in_the_system_id_does_not_hide_entities() {
        let doc = br#"<!DOCTYPE svg SYSTEM "a>b.dtd" [<!ENTITY e "<g><g/></g>">]><svg>&e;</svg>"#;
        assert_eq!(check(doc, MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Entities));
    }

    /// The same scan locked onto the first `<!DOCTYPE` in the byte stream, so a decoy inside a
    /// comment hid the real declaration behind it. Also from that review.
    #[test]
    fn a_doctype_decoy_in_a_comment_does_not_hide_entities() {
        let doc = br#"<!-- <!DOCTYPE decoy> --><!DOCTYPE svg [<!ENTITY e "<g><g/></g>">]><svg>&e;</svg>"#;
        assert_eq!(check(doc, MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Entities));
    }

    #[test]
    fn lower_case_entity_declarations_are_refused_too() {
        // XML requires upper case, so this is malformed either way - but over-matching here costs
        // nothing and means the check does not depend on the parser being strict about it.
        let doc = br#"<!DOCTYPE svg [<!entity e "<g><g/></g>">]><svg>&e;</svg>"#;
        assert_eq!(check(doc, MAX_SVG_NESTING_DEPTH), Some(XmlLimit::Entities));
    }

    #[test]
    fn the_text_entity_somewhere_harmless_is_not_a_declaration() {
        // Both of these are valid SVG that merely mentions the word, and neither declares
        // anything. Comments and quoted attribute values are skipped whole.
        assert_eq!(
            check(br#"<svg><!-- <!ENTITY e "<g><g/></g>"> --><rect/></svg>"#, 4),
            None
        );
        assert_eq!(check(br#"<svg><desc title="<!ENTITY e '<g/>'>"/></svg>"#, 4), None);
        assert_eq!(check(br#"<svg><![CDATA[<!ENTITY e "<g><g/></g>">]]></svg>"#, 4), None);
    }
}
