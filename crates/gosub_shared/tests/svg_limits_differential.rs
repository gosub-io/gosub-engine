//! Differential test: does [`xml_exceeds_limits`] measure nesting depth the way the real parser
//! does?

use gosub_shared::svg_limits::{xml_exceeds_limits, XmlLimit};

/// Depth the parser really builds, or `None` if it refuses the document outright - in which case
/// it never recurses into it, so whatever the scanner said is harmless.
fn real_depth(xml: &str) -> Option<usize> {
    let opt = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, opt).ok()?;
    doc.descendants()
        .filter(roxmltree::Node::is_element)
        .map(|n| n.ancestors().filter(roxmltree::Node::is_element).count())
        .max()
        .or(Some(0))
}

/// See the module note on why this is not `MAX_SVG_NESTING_DEPTH`.
const LIMIT: usize = 20;

fn cases() -> Vec<(&'static str, String)> {
    // Deep enough to cross LIMIT when it counts, shallow enough for the parser to survive.
    let deep = "<g>".repeat(40) + &"</g>".repeat(40);
    vec![
        ("comment holding markup", format!("<svg><!-- {deep} --><rect/></svg>")),
        (
            "comment ending at its first -->",
            format!("<svg><!-- x --> {deep} <rect/></svg>"),
        ),
        ("cdata holding markup", format!("<svg><![CDATA[{deep}]]><rect/></svg>")),
        (
            "cdata with a stray ]] inside",
            format!("<svg><![CDATA[ ]] {deep} ]]><rect/></svg>"),
        ),
        (
            "cdata containing a comment",
            format!("<svg><![CDATA[<!-- {deep} -->]]><rect/></svg>"),
        ),
        (
            "comment containing a cdata opener",
            format!("<svg><!-- <![CDATA[ {deep} --><rect/></svg>"),
        ),
        (
            "processing instruction holding markup",
            format!("<svg><?pi {deep} ?><rect/></svg>"),
        ),
        (
            "processing instruction with a stray ?",
            format!("<svg><?pi a?b {deep} ?><rect/></svg>"),
        ),
        (
            "attribute value holding markup",
            format!("<svg><desc t=\"{deep}\"/><rect/></svg>"),
        ),
        (
            "attribute value holding > and /",
            "<svg><a href=\"a>b/\"><g/></a></svg>".to_string(),
        ),
        (
            "doctype with a quoted > in its system id",
            format!("<!DOCTYPE svg SYSTEM \"a>b\"><svg>{deep}<rect/></svg>"),
        ),
        (
            "doctype with an internal subset",
            format!("<!DOCTYPE svg [<!ELEMENT g ANY>]><svg>{deep}<rect/></svg>"),
        ),
        (
            "escaped markup in text",
            format!("<svg><text>a &lt; b {deep}</text></svg>"),
        ),
        (
            "self-closing tags do not nest",
            format!("<svg>{}</svg>", "<rect/>".repeat(60)),
        ),
        (
            "xml declaration then a comment",
            format!("<?xml version=\"1.0\"?><!-- {deep} --><svg><rect/></svg>"),
        ),
        (
            "entity declared with markup in it",
            format!("<!DOCTYPE svg [<!ENTITY e \"{deep}\">]><svg>&e;</svg>"),
        ),
        (
            "entity declared without markup",
            "<!DOCTYPE svg [<!ENTITY e \"fill:red\">]><svg><rect style=\"&e;\"/></svg>".to_string(),
        ),
    ]
}

#[test]
fn the_scanner_agrees_with_the_parser() {
    let mut bypassed = Vec::new();
    let mut over_rejected = Vec::new();

    for (name, xml) in cases() {
        let scanned = xml_exceeds_limits(xml.as_bytes(), LIMIT);
        let Some(real) = real_depth(&xml) else {
            continue;
        };

        match scanned {
            // Refusing an entity declaration is policy, not a depth measurement: its expansion is
            // what the scanner declines to predict. The unit tests cover that split.
            Some(XmlLimit::Entities) => {}
            Some(XmlLimit::Depth) if real <= LIMIT => over_rejected.push((name, real)),
            Some(XmlLimit::Depth) => {}
            None if real > LIMIT => bypassed.push((name, real)),
            None => {}
        }
    }

    assert!(
        bypassed.is_empty(),
        "the scanner accepted documents the parser nests deeper than {LIMIT}, which is the bypass \
         this guard exists to prevent: {bypassed:?}"
    );
    assert!(
        over_rejected.is_empty(),
        "the scanner refused documents that are actually within {LIMIT}; not a security problem, \
         but it costs compatibility and is currently never the case: {over_rejected:?}"
    );
}
