//! Gemtext (`text/gemini`) as an HTML page.
//!
//! Gemtext is line based: what a line is follows from how it starts, and a line never runs on
//! into the next. `=>` is a link, `#`, `##` and `###` headings, `* ` a list item, `>` a quote,
//! and a line of three backticks opens or closes a preformatted block, with what follows the
//! backticks of an opening line as the block's description. Every other line is text, an
//! empty one a gap.
//!
//! A link is followed only to where a gemini page may go: another gemini page, or the web.
//! Any other link is shown with its address, as text. Everything the server wrote goes
//! through [`escape`].

use crate::net::protocols::page::{self, escape, title_for};
use url::Url;

pub(super) const STYLE: &str = "body{margin:0;padding:16px 24px;font-family:sans-serif;line-height:1.5;max-width:46em}\
p,ul,blockquote,pre{margin:0}h1,h2,h3{margin:.5em 0 .25em}\
pre{font-family:monospace;line-height:1.3;white-space:pre;overflow-x:auto}\
blockquote{padding-left:12px;border-left:3px solid #999;font-style:italic}\
.gap{height:1em}.u{color:#666}";

/// The page for gemtext `text` at `url`, in `lang` when the server named one.
pub fn page(url: &Url, text: &str, lang: Option<&str>) -> String {
    let mut body = String::with_capacity(text.len() * 2);
    let mut title = None;
    let mut list = false;
    let mut pre = false;
    for line in text.lines() {
        if pre {
            if line.starts_with("```") {
                body.push_str("</pre>");
                pre = false;
            } else {
                body.push_str(&escape(line));
                body.push('\n');
            }
            continue;
        }
        let item = line.strip_prefix("* ");
        if list && item.is_none() {
            body.push_str("</ul>");
            list = false;
        }
        if let Some(alt) = line.strip_prefix("```") {
            let alt = alt.trim();
            if alt.is_empty() {
                body.push_str("<pre>");
            } else {
                body.push_str(&format!("<pre aria-label=\"{}\">", escape(alt)));
            }
            pre = true;
        } else if let Some(link) = line.strip_prefix("=>") {
            body.push_str(&link_line(url, link));
        } else if let Some((level, heading)) = heading(line) {
            if level == 1 && title.is_none() && !heading.is_empty() {
                title = Some(heading.to_string());
            }
            body.push_str(&format!("<h{level}>{}</h{level}>", escape(heading)));
        } else if let Some(item) = item {
            if !list {
                body.push_str("<ul>");
                list = true;
            }
            body.push_str(&format!("<li>{}</li>", escape(item.trim())));
        } else if let Some(quote) = line.strip_prefix('>') {
            body.push_str(&format!("<blockquote>{}</blockquote>", escape(quote.trim())));
        } else if line.trim().is_empty() {
            body.push_str("<div class=\"gap\"></div>");
        } else {
            body.push_str(&format!("<p>{}</p>", escape(line)));
        }
    }
    // A response may end inside a block or a list.
    if pre {
        body.push_str("</pre>");
    }
    if list {
        body.push_str("</ul>");
    }
    let title = title.unwrap_or_else(|| title_for(url));
    page::document_in(lang, &title, STYLE, &body)
}

/// A heading line's level and text.
fn heading(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    (1..=3).contains(&level).then(|| (level as u8, line[level..].trim()))
}

/// A link line, after its `=>`: the address, then whitespace and a label, or the address alone.
fn link_line(base: &Url, link: &str) -> String {
    let link = link.trim();
    let (target, label) = match link.split_once(char::is_whitespace) {
        Some((target, label)) => (target, label.trim()),
        None => (link, ""),
    };
    if target.is_empty() {
        return String::new();
    }
    let label = if label.is_empty() { target } else { label };
    match base.join(target) {
        Ok(to) if matches!(to.scheme(), "gemini" | "http" | "https") => {
            format!("<p><a href=\"{}\">{}</a></p>", escape(to.as_str()), escape(label))
        }
        _ if label == target => format!("<p class=\"u\">{}</p>", escape(target)),
        _ => format!("<p>{} <span class=\"u\">{}</span></p>", escape(label), escape(target)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(text: &str) -> String {
        let page = page(&Url::parse("gemini://h.test/dir/page.gmi").unwrap(), text, None);
        let body = page.split("<body>").nth(1).unwrap();
        body.strip_suffix("</body></html>").unwrap().to_string()
    }

    #[test]
    fn each_line_type_has_its_element() {
        assert_eq!(
            render("# One\n## Two\n### Three\ntext\n\n> quoted\n* a\n* b\nafter"),
            "<h1>One</h1><h2>Two</h2><h3>Three</h3><p>text</p><div class=\"gap\"></div>\
<blockquote>quoted</blockquote><ul><li>a</li><li>b</li></ul><p>after</p>"
        );
    }

    #[test]
    fn four_hashes_and_a_star_without_a_space_are_text() {
        assert_eq!(render("#### x\n*x"), "<p>#### x</p><p>*x</p>");
    }

    #[test]
    fn links_resolve_against_the_page_and_go_only_to_gemini_or_the_web() {
        assert_eq!(
            render("=> other.gmi Other page\n=>/top\n=> https://w.test/ The web"),
            "<p><a href=\"gemini://h.test/dir/other.gmi\">Other page</a></p>\
<p><a href=\"gemini://h.test/top\">/top</a></p>\
<p><a href=\"https://w.test/\">The web</a></p>"
        );
        assert_eq!(
            render("=> gopher://g.test/1/ A hole\n=> file:///etc/passwd"),
            "<p>A hole <span class=\"u\">gopher://g.test/1/</span></p><p class=\"u\">file:///etc/passwd</p>"
        );
        assert_eq!(render("=>"), "");
    }

    #[test]
    fn a_preformatted_block_is_kept_verbatim_with_its_description() {
        assert_eq!(
            render("```ascii art\n# not a heading\n  => <b>\n```\ntext"),
            "<pre aria-label=\"ascii art\"># not a heading\n  =&gt; &lt;b&gt;\n</pre><p>text</p>"
        );
        assert_eq!(render("```\nx"), "<pre>x\n</pre>");
    }

    #[test]
    fn server_text_is_never_markup() {
        assert_eq!(
            render("<script>x</script>\n=> /\"onclick=\"x <img>"),
            "<p>&lt;script&gt;x&lt;/script&gt;</p><p><a href=\"gemini://h.test/%22onclick=%22x\">&lt;img&gt;</a></p>"
        );
    }

    #[test]
    fn the_title_is_the_first_top_heading() {
        let url = Url::parse("gemini://h.test/").unwrap();
        assert!(page(&url, "## sub\n# Main\n# Later", None).contains("<title>Main</title>"));
        assert!(page(&url, "text", None).contains("<title>h.test/</title>"));
    }

    #[test]
    fn crlf_line_ends_are_line_ends() {
        assert_eq!(
            render("# A\r\nb\r\n\r\nc\n"),
            "<h1>A</h1><p>b</p><div class=\"gap\"></div><p>c</p>"
        );
    }
}
