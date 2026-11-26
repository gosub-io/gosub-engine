//! Tree construction in foreign content (SVG and MathML).

use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
use crate::parser::attr_replacements::{
    MATHML_ADJUSTMENTS, SVG_ADJUSTMENTS_ATTRIBUTES, SVG_ADJUSTMENTS_TAGS, XML_ADJUSTMENTS,
};
use crate::parser::helper::{is_html_integration_point, is_mathml_text_integration_point};
use crate::parser::{Delegate, Html5Parser};
use crate::tokenizer::token::Token;
use crate::tokenizer::CHAR_REPLACEMENT;
use cow_utils::CowUtils;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;

/// Whether `adjusted_attribute_name` renames `name`, without building the new name.
fn is_adjusted_attribute(table: Option<&phf::Map<&str, &str>>, name: &str) -> bool {
    table.is_some_and(|t| t.contains_key(name)) || XML_ADJUSTMENTS.contains_key(name)
}

/// The name `name` gets from `table` (MathML or SVG) and then from the foreign attribute
/// adjustments, or `None` when neither renames it.
fn adjusted_attribute_name(table: Option<&phf::Map<&str, &str>>, name: &str) -> Option<String> {
    let renamed = table.and_then(|t| t.get(name).copied());
    if let Some((prefix, local_name, _namespace)) = XML_ADJUSTMENTS.get(renamed.unwrap_or(name)) {
        return Some(format!("{prefix} {local_name}"));
    }
    renamed.map(str::to_owned)
}

impl<C: HasDocument> Html5Parser<'_, C> {
    // Process token in foreign content (svg, mathml)
    pub(crate) fn process_foreign_content(&mut self) {
        // Move the token out as `process_html_content` does. The arms that hand the token
        // to HTML processing only set `delegate`; the token is put back before that call,
        // as `process_html_content` re-reads `self.current_token`.
        let loc = self.current_token.get_location();
        let mut current_token = std::mem::replace(&mut self.current_token, Token::Eof { location: loc });

        let mut handle_as_script_endtag = false;
        let mut delegate = Delegate::None;

        match &current_token {
            Token::Text { text: value, location } if current_token.is_mixed() => {
                let tokens = self.split_mixed_token(value, *location);
                self.tokenizer.insert_tokens_at_queue_start(tokens);
                return;
            }
            Token::Text { text: value, .. } if current_token.is_null() => {
                self.null_parse_errors(value, loc, "null character not allowed in foreign content");
                // A run of NULs arrives as one token; each NUL gets its own replacement
                self.insert_text_element(&Token::Text {
                    text: CHAR_REPLACEMENT.to_string().repeat(value.chars().count()),
                    location: loc,
                });
            }
            Token::Text { .. } if current_token.is_empty_or_white() => {
                self.insert_text_element(&current_token);
            }
            Token::Text { .. } => {
                self.insert_text_element(&current_token);

                self.frameset_ok = false;
            }
            Token::Comment { .. } => {
                self.insert_comment_element(&current_token, None);
            }
            Token::DocType { .. } => {
                self.parse_error("doctype not allowed in foreign content");
                // ignore token
            }
            Token::StartTag { name, .. }
                if name == "b"
                    || name == "big"
                    || name == "blockquote"
                    || name == "body"
                    || name == "br"
                    || name == "center"
                    || name == "code"
                    || name == "dd"
                    || name == "div"
                    || name == "dl"
                    || name == "dt"
                    || name == "em"
                    || name == "embed"
                    || name == "h1"
                    || name == "h2"
                    || name == "h3"
                    || name == "h4"
                    || name == "h5"
                    || name == "h6"
                    || name == "head"
                    || name == "hr"
                    || name == "i"
                    || name == "img"
                    || name == "li"
                    || name == "listing"
                    || name == "menu"
                    || name == "meta"
                    || name == "nobr"
                    || name == "ol"
                    || name == "p"
                    || name == "pre"
                    || name == "ruby"
                    || name == "s"
                    || name == "small"
                    || name == "span"
                    || name == "strong"
                    || name == "strike"
                    || name == "sub"
                    || name == "sup"
                    || name == "table"
                    || name == "tt"
                    || name == "u"
                    || name == "ul"
                    || name == "var" =>
            {
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::StartTag { name, attributes, .. }
                if name == "font"
                    && (attributes.contains_key("color")
                        || attributes.contains_key("face")
                        || attributes.contains_key("size")) =>
            {
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::EndTag { name, .. } if name == "br" || name == "p" => {
                delegate = Delegate::UnexpectedHtmlTag;
            }
            Token::StartTag {
                name, is_self_closing, ..
            } => {
                let is_self_closing = *is_self_closing;
                let is_script = name == "script";

                let acn_id = self.get_adjusted_current_node_id();
                let acn_ns = self.document.namespace(acn_id).unwrap_or(HTML_NAMESPACE).to_owned();
                if acn_ns == SVG_NAMESPACE {
                    self.adjust_svg_tag_names(&mut current_token);
                }
                self.adjust_foreign_attributes(&mut current_token, &acn_ns);
                self.insert_foreign_element(&mut current_token, &acn_ns);

                if is_self_closing {
                    let cn_id = current_node_id!(self);
                    if is_script && node_ns!(self, cn_id) == Some(SVG_NAMESPACE) {
                        self.ack_self_closing = true;
                        handle_as_script_endtag = true;
                    } else {
                        self.open_elements.pop();
                        self.ack_self_closing = true;
                    }
                }
            }
            Token::EndTag { name, .. }
                if name == "script"
                    && node_tag!(self, current_node_id!(self)) == "script"
                    && node_ns!(self, current_node_id!(self)) == Some(SVG_NAMESPACE) =>
            {
                handle_as_script_endtag = true;
            }
            Token::EndTag { name, .. } => {
                if self.open_elements.is_empty() {
                    return;
                }

                let mut node_idx = self.open_elements.len() - 1;
                let mut cur_node_id = self.open_elements[node_idx];

                if node_tag!(self, cur_node_id).cow_to_lowercase() != *name {
                    self.parse_error("end tag does not match current node");
                }

                loop {
                    // Fragment case is when the first element in the stack is this node
                    match self.open_elements.first() {
                        // fragment case
                        Some(first_id) if *first_id == cur_node_id => return,
                        _ => {}
                    }

                    if node_tag!(self, cur_node_id).cow_to_lowercase() == *name {
                        while let Some(node_id) = self.open_elements.pop() {
                            if node_id == cur_node_id {
                                break;
                            }
                        }
                        return;
                    }

                    node_idx -= 1;
                    cur_node_id = self.open_elements[node_idx];

                    if node_ns!(self, cur_node_id) != Some(HTML_NAMESPACE) {
                        continue;
                    }

                    delegate = Delegate::HtmlContent;
                    break;
                }
            }
            Token::Eof { .. } => {
                return;
            }
        }

        match delegate {
            Delegate::None => {}
            Delegate::UnexpectedHtmlTag => {
                self.current_token = current_token;
                self.process_unexpected_html_tag();
                return;
            }
            Delegate::HtmlContent => {
                self.current_token = current_token;
                self.process_html_content();
                return;
            }
        }

        if handle_as_script_endtag {
            let script = current_node_id!(self);
            self.open_elements.pop();

            let old_insertion_point = self.insertion_point;
            self.insertion_point = Some(self.tokenizer.get_location().offset);

            self.script_nesting_level += 1;

            let ran = self.run_script(script);

            self.script_nesting_level -= 1;
            if self.script_nesting_level == 0 {
                self.parser_pause_flag = false;
                if ran {
                    self.script_checkpoint();
                }
            }

            self.insertion_point = old_insertion_point;
        }
    }

    /// Finds the node where to place an unexpected html tag. This can only be done on a mathml
    /// insertion point, a `svg_html` insertion point, or at a regular html namespaced node.
    pub(crate) fn process_unexpected_html_tag(&mut self) {
        self.parse_error("process_unexpected_html_tag");

        loop {
            let cn_id = current_node_id!(self);
            if is_mathml_text_integration_point::<C>(self.document, cn_id)
                || is_html_integration_point::<C>(self.document, cn_id)
                || self.document.namespace(cn_id) == Some(HTML_NAMESPACE)
            {
                break;
            }
            self.open_elements.pop();
            if self.open_elements.is_empty() {
                return;
            }
        }

        // Process as HTML content
        self.process_html_content();
    }

    /// Adjusts tag name in the given token for SVG
    pub(crate) fn adjust_svg_tag_names(&self, token: &mut Token) {
        if let Token::StartTag { name, .. } = token {
            if let Some(new_name) = SVG_ADJUSTMENTS_TAGS.get(name) {
                (*new_name).clone_into(name);
            }
        }
    }

    /// Adjusts the attribute names of a foreign start tag: the MathML or SVG adjustments
    /// for `namespace`, then the foreign (xlink, xml, xmlns) ones. The attributes are only
    /// rebuilt when a name changes.
    pub(crate) fn adjust_foreign_attributes(&self, token: &mut Token, namespace: &str) {
        let Token::StartTag { attributes, .. } = token else {
            return;
        };
        let table = if namespace == MATHML_NAMESPACE {
            Some(&MATHML_ADJUSTMENTS)
        } else if namespace == SVG_NAMESPACE {
            Some(&SVG_ADJUSTMENTS_ATTRIBUTES)
        } else {
            None
        };
        if attributes.keys().all(|name| !is_adjusted_attribute(table, name)) {
            return;
        }
        *attributes = std::mem::take(attributes)
            .into_iter()
            .map(|(name, value)| (adjusted_attribute_name(table, &name).unwrap_or(name), value))
            .collect();
    }
}

#[cfg(test)]
mod foreign_content_tests {
    use crate::document::document_impl::DocumentImpl;
    use crate::node::{HTML_NAMESPACE, MATHML_NAMESPACE, SVG_NAMESPACE};
    use crate::parser::Html5Parser;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_interface::document::Document;
    use gosub_shared::node::NodeId;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;
    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    /// First element with this tag name, depth first.
    fn find(doc: &DocumentImpl<Config>, id: NodeId, tag: &str) -> Option<NodeId> {
        if doc.tag_name(id) == Some(tag) {
            return Some(id);
        }
        doc.children(id).iter().find_map(|&c| find(doc, c, tag))
    }

    #[test]
    fn foreign_start_tags_get_adjusted_names_and_attributes() {
        let doc = crate::html_compile::<Config>(
            r##"<svg viewbox="0 0 1 1"><foreignobject></foreignobject><path xlink:href="#a"/></svg>
                <math definitionurl="u"></math>"##,
        );
        let svg = find(&doc, NodeId::root(), "svg").unwrap();
        assert_eq!(doc.namespace(svg), Some(SVG_NAMESPACE));
        assert_eq!(doc.attribute(svg, "viewBox"), Some("0 0 1 1"));
        assert!(find(&doc, svg, "foreignObject").is_some());
        let path = find(&doc, svg, "path").unwrap();
        assert_eq!(doc.attribute(path, "xlink href"), Some("#a"));

        let math = find(&doc, NodeId::root(), "math").unwrap();
        assert_eq!(doc.namespace(math), Some(MATHML_NAMESPACE));
        assert_eq!(doc.attribute(math, "definitionURL"), Some("u"));
    }

    /// Names of the attributes of `id`, in document order.
    fn attribute_names(doc: &DocumentImpl<Config>, id: NodeId) -> Vec<String> {
        doc.attributes(id).unwrap().keys().cloned().collect()
    }

    /// Renaming keeps the attributes in source order, in every place foreign attributes are
    /// adjusted: `<svg>` and `<math>` in body, and start tags inside foreign content.
    #[test]
    fn adjusted_foreign_attributes_keep_their_order() {
        let doc = crate::html_compile::<Config>(
            r##"<svg a="1" viewbox="v" xlink:href="#h" b="2"><path d="x" xml:lang="en" pathlength="3"/>
                <foreignObject><math c="3" definitionurl="u" xlink:show="s"></math></foreignObject></svg>
                <math definitionurl="u" z="1" xlink:href="#m"><mi viewbox="v" definitionurl="d"/></math>"##,
        );
        let svg = find(&doc, NodeId::root(), "svg").unwrap();
        assert_eq!(attribute_names(&doc, svg), ["a", "viewBox", "xlink href", "b"]);
        let path = find(&doc, svg, "path").unwrap();
        assert_eq!(attribute_names(&doc, path), ["d", "xml lang", "pathLength"]);

        let math_in_svg = find(&doc, svg, "math").unwrap();
        assert_eq!(attribute_names(&doc, math_in_svg), ["c", "definitionURL", "xlink show"]);

        let body = find(&doc, NodeId::root(), "body").unwrap();
        let math = doc
            .children(body)
            .iter()
            .copied()
            .find(|&c| doc.tag_name(c) == Some("math"))
            .unwrap();
        assert_eq!(attribute_names(&doc, math), ["definitionURL", "z", "xlink href"]);
        let mi = find(&doc, math, "mi").unwrap();
        assert_eq!(attribute_names(&doc, mi), ["viewbox", "definitionURL"]);
    }

    /// Tokens handed from foreign content to HTML processing must be the real token.
    #[test]
    fn tokens_handed_back_to_html_keep_their_contents() {
        let doc = crate::html_compile::<Config>(
            r#"<svg><b class="x">t</b></svg><svg><font color="red"></font></svg><div><svg><g></div>after"#,
        );
        let b = find(&doc, NodeId::root(), "b").unwrap();
        assert_eq!(doc.namespace(b), Some(HTML_NAMESPACE));
        assert_eq!(doc.attribute(b, "class"), Some("x"));

        let font = find(&doc, NodeId::root(), "font").unwrap();
        assert_eq!(doc.namespace(font), Some(HTML_NAMESPACE));
        assert_eq!(doc.attribute(font, "color"), Some("red"));

        let body = find(&doc, NodeId::root(), "body").unwrap();
        let last = *doc.children(body).last().unwrap();
        assert_eq!(doc.text_value(last), Some("after"));
    }

    /// Text of the direct text children of `id`, joined.
    fn text_of(doc: &DocumentImpl<Config>, id: NodeId) -> String {
        doc.children(id).iter().filter_map(|&c| doc.text_value(c)).collect()
    }

    /// Every NUL in foreign content becomes its own U+FFFD, not one per run.
    #[test]
    fn each_null_in_foreign_content_becomes_a_replacement_character() {
        let cases = [
            ("<svg>a\0b", "svg", "a\u{FFFD}b"),
            ("<svg>a\0\0b", "svg", "a\u{FFFD}\u{FFFD}b"),
            ("<svg>a\0\0\0b", "svg", "a\u{FFFD}\u{FFFD}\u{FFFD}b"),
            ("<svg>\0\0a", "svg", "\u{FFFD}\u{FFFD}a"),
            ("<svg>a\0\0", "svg", "a\u{FFFD}\u{FFFD}"),
            ("<svg>\0\0", "svg", "\u{FFFD}\u{FFFD}"),
            ("<svg>\0\0 \0\0", "svg", "\u{FFFD}\u{FFFD} \u{FFFD}\u{FFFD}"),
            ("<svg> \0\0 a", "svg", " \u{FFFD}\u{FFFD} a"),
            ("<svg><![CDATA[a\0\0b]]>", "svg", "a\u{FFFD}\u{FFFD}b"),
            ("<math>a\0\0b", "math", "a\u{FFFD}\u{FFFD}b"),
            ("<math>\0\0 \0", "math", "\u{FFFD}\u{FFFD} \u{FFFD}"),
            ("<svg><g>a\0\0b</g>", "g", "a\u{FFFD}\u{FFFD}b"),
        ];
        for (input, tag, expected) in cases {
            let doc = crate::html_compile::<Config>(input);
            let id = find(&doc, NodeId::root(), tag).unwrap();
            assert_eq!(text_of(&doc, id), expected, "input {input:?}");
        }
    }

    /// Text in an integration point is HTML content, where the in body rules drop NULs.
    #[test]
    fn nulls_in_integration_points_are_dropped() {
        let cases = [
            ("<math><mi>a\0\0b", "mi", "ab"),
            ("<math><mi>\0", "mi", ""),
            ("<svg><foreignObject>a\0\0b", "foreignObject", "ab"),
            ("<svg><desc>a\0\0\0b", "desc", "ab"),
        ];
        for (input, tag, expected) in cases {
            let doc = crate::html_compile::<Config>(input);
            let id = find(&doc, NodeId::root(), tag).unwrap();
            assert_eq!(text_of(&doc, id), expected, "input {input:?}");
        }
    }
}
