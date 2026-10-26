//! HTML pages an engine protocol builds from what a server sent.
//!
//! Everything that came from a server goes through [`escape`]: it is text, never markup.

/// A whole page: `title` and `body` in a document with `style`. The title is escaped here;
/// `body` is markup the protocol built, with its server text already escaped.
pub fn document(title: &str, style: &str, body: &str) -> String {
    document_in(None, title, style, body)
}

/// [`document`], in the language `lang` names when it is a language tag. A server's tag is
/// kept to the characters one has, so it never leaves the attribute.
pub fn document_in(lang: Option<&str>, title: &str, style: &str, body: &str) -> String {
    let lang = lang
        .filter(|l| !l.is_empty() && l.len() <= 35 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .map(|l| format!(" lang=\"{l}\""))
        .unwrap_or_default();
    format!(
        "<!DOCTYPE html><html{lang}><head><meta charset=\"utf-8\"><title>{}</title><style>{style}</style></head>\
<body>{body}</body></html>",
        escape(title)
    )
}

/// `s` as text in an element or a double-quoted attribute.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A page's title: where it is, without the scheme.
pub fn title_for(url: &url::Url) -> String {
    let s = url.as_str();
    s.strip_prefix(url.scheme())
        .and_then(|rest| rest.strip_prefix("://"))
        .unwrap_or(s)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_escaped() {
        assert_eq!(
            escape("<a href=\"x\">&</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn the_title_is_escaped_and_the_body_is_not() {
        let page = document("<t>", "", "<pre>x</pre>");
        assert!(page.contains("<title>&lt;t&gt;</title>"));
        assert!(page.contains("<body><pre>x</pre></body>"));
    }

    #[test]
    fn only_a_language_tag_becomes_the_lang() {
        assert!(document_in(Some("en-GB"), "", "", "").starts_with("<!DOCTYPE html><html lang=\"en-GB\"><head>"));
        for bad in ["en\" onload=\"x", "", "en_GB"] {
            assert!(
                document_in(Some(bad), "", "", "").starts_with("<!DOCTYPE html><html><head>"),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_title_drops_the_scheme() {
        assert_eq!(
            title_for(&url::Url::parse("finger://h.test/bob").unwrap()),
            "h.test/bob"
        );
        assert_eq!(
            title_for(&url::Url::parse("gopher://h.test:7070/1/").unwrap()),
            "h.test:7070/1/"
        );
    }
}
