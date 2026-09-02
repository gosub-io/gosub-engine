//! Text helpers the DOM getters need.
//!
//! These belong in shared engine code once scripting is real - `engine::form::option_value`
//! already grows its own (trim-only) variant of `strip_and_collapse`.

use gosub_interface::document::Document as _;
use gosub_shared::node::NodeId;

use crate::Doc;

/// Concatenated data of every Text descendant of `id`, in tree order.
pub fn descendant_text(doc: &Doc, id: NodeId) -> String {
    let mut out = String::new();
    collect(doc, id, &mut out);
    out
}

fn collect(doc: &Doc, id: NodeId, out: &mut String) {
    if let Some(text) = doc.text_value(id) {
        out.push_str(text);
    }
    for &child in doc.children(id) {
        collect(doc, child, out);
    }
}

/// Infra's "strip and collapse ASCII whitespace": trim the ends, and squeeze every inner run
/// of whitespace down to a single space.
pub fn strip_and_collapse(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_space = false;
    for ch in input.chars() {
        if ch.is_ascii_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_and_collapse;

    #[test]
    fn collapses_inner_runs_and_trims_the_ends() {
        assert_eq!(strip_and_collapse(" child "), "child");
        assert_eq!(strip_and_collapse(" child  node "), "child node");
        assert_eq!(strip_and_collapse("\n\ta\r\n b\t"), "a b");
        assert_eq!(strip_and_collapse("   "), "");
    }
}
