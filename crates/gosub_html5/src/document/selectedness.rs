//! The selectedness of `<option>` elements: whether each one is selected, kept up to date as the
//! tree and the attributes change (<https://html.spec.whatwg.org/#concept-option-selectedness>).
//!
//! Selectedness is state, not a function of the markup: disabling the option a select fell back
//! to leaves it selected, and only the changes the spec names run the selectedness setting
//! algorithm again. `DocumentImpl` calls in here from every tree and attribute change.
//!
//! `selectedcontent` elements and the `Option()` constructor are not supported, so nothing here
//! reports whether a change happened. Neither is dirtiness: nothing sets it yet, as there is no
//! `option.selected` setter and a pick by the user is kept by the engine
//! (`InteractionState::selected`), not here.

use crate::document::presentation_hints::non_negative_integer;
use crate::node::HTML_NAMESPACE;
use gosub_interface::config::HasDocument;
use gosub_interface::document::Document;
use gosub_shared::node::NodeId;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct OptionState {
    selected: bool,
    /// The select whose list of options it is in, as of the last change to its ancestors (the
    /// spec's cached nearest ancestor select element).
    select: Option<NodeId>,
}

/// The state of every `<option>` of a document, and per select the options in its list that
/// are selected. Every write goes through the methods here, so the two cannot drift apart.
#[derive(Debug, Clone, Default)]
pub(crate) struct Selectedness {
    options: HashMap<NodeId, OptionState>,
    /// Select -> the options in its list that are selected. A select without `multiple` has at
    /// most one once a change is done; the setting algorithm sees to that.
    selected: HashMap<NodeId, HashSet<NodeId>>,
    /// Selects whose list of options was last found to hold no option that is not disabled. An
    /// option that joins one of these is the only one that can be selected, so the setting
    /// algorithm does not walk the list again; without this a select that starts with many
    /// disabled options is quadratic to parse. Dropped whenever an option already in the list
    /// may have become enabled.
    without_enabled: HashSet<NodeId>,
}

impl Selectedness {
    /// A new option, in no select yet: selected when it has the `selected` attribute.
    pub(crate) fn add(&mut self, option: NodeId, selected: bool) {
        self.options.insert(
            option,
            OptionState {
                selected,
                ..OptionState::default()
            },
        );
    }

    /// Forget a deleted option. Returns the select it was in.
    pub(crate) fn remove(&mut self, option: NodeId) -> Option<NodeId> {
        let select = self.set_select(option, None);
        self.options.remove(&option);
        select
    }

    pub(crate) fn is_option(&self, id: NodeId) -> bool {
        self.options.contains_key(&id)
    }

    pub(crate) fn is_selected(&self, option: NodeId) -> bool {
        self.options.get(&option).is_some_and(|o| o.selected)
    }

    pub(crate) fn select_of(&self, option: NodeId) -> Option<NodeId> {
        self.options.get(&option).and_then(|o| o.select)
    }

    /// The selected options in `select`'s list, in no particular order.
    pub(crate) fn selected_in(&self, select: NodeId) -> Vec<NodeId> {
        self.selected
            .get(&select)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn set_selected(&mut self, option: NodeId, selected: bool) {
        let Some(state) = self.options.get_mut(&option) else {
            return;
        };
        state.selected = selected;
        if let Some(select) = state.select {
            self.index(select, option, selected);
        }
    }

    /// Point `option` at the select it is in now. Returns the one it was in.
    pub(crate) fn set_select(&mut self, option: NodeId, select: Option<NodeId>) -> Option<NodeId> {
        let state = self.options.get_mut(&option)?;
        let old = std::mem::replace(&mut state.select, select);
        let selected = state.selected;
        if old != select && selected {
            if let Some(old) = old {
                self.index(old, option, false);
            }
            if let Some(select) = select {
                self.index(select, option, true);
            }
        }
        old
    }

    pub(crate) fn has_no_enabled(&self, select: NodeId) -> bool {
        self.without_enabled.contains(&select)
    }

    pub(crate) fn set_has_no_enabled(&mut self, select: NodeId, none: bool) {
        if none {
            self.without_enabled.insert(select);
        } else {
            self.without_enabled.remove(&select);
        }
    }

    fn index(&mut self, select: NodeId, option: NodeId, selected: bool) {
        if selected {
            self.selected.entry(select).or_default().insert(option);
        } else if let Some(set) = self.selected.get_mut(&select) {
            set.remove(&option);
            if set.is_empty() {
                self.selected.remove(&select);
            }
        }
    }
}

/// Whether `id` is the HTML element `name`. An SVG or MathML `option` is not an option.
pub(crate) fn is_html<C: HasDocument>(doc: &C::Document, id: NodeId, name: &str) -> bool {
    doc.tag_name(id) == Some(name) && doc.namespace(id) == Some(HTML_NAMESPACE)
}

/// The tag of `id` when it is an HTML element.
fn html_tag<C: HasDocument>(doc: &C::Document, id: NodeId) -> Option<&str> {
    doc.tag_name(id).filter(|_| doc.namespace(id) == Some(HTML_NAMESPACE))
}

/// Whether the children of `id` are template contents, which are in no tree of the document.
fn holds_template_contents<C: HasDocument>(doc: &C::Document, id: NodeId) -> bool {
    doc.template_contents(id) == Some(id)
}

/// The select whose list of options `element` would be in
/// (<https://html.spec.whatwg.org/#option-element-nearest-ancestor-select>).
pub(crate) fn nearest_ancestor_select<C: HasDocument>(doc: &C::Document, element: NodeId) -> Option<NodeId> {
    let mut seen_optgroup = false;
    let mut current = element;
    while let Some(ancestor) = doc.parent(current) {
        if holds_template_contents::<C>(doc, ancestor) {
            return None;
        }
        match html_tag::<C>(doc, ancestor) {
            Some("datalist" | "hr" | "option") => return None,
            Some("optgroup") if seen_optgroup => return None,
            Some("optgroup") => seen_optgroup = true,
            Some("select") => return Some(ancestor),
            _ => {}
        }
        current = ancestor;
    }
    None
}

/// `select`'s list of options, in tree order
/// (<https://html.spec.whatwg.org/#concept-select-option-list>).
pub(crate) fn list_of_options<C: HasDocument>(doc: &C::Document, select: NodeId) -> Vec<NodeId> {
    let mut options = Vec::new();
    if holds_template_contents::<C>(doc, select) {
        return options;
    }
    // Each node with whether an optgroup lies between it and the select.
    let mut stack: Vec<(NodeId, bool)> = doc.children(select).iter().rev().map(|&id| (id, false)).collect();
    while let Some((id, in_optgroup)) = stack.pop() {
        let tag = html_tag::<C>(doc, id);
        if tag == Some("option") {
            options.push(id);
        }
        let skip = match tag {
            Some("select" | "hr" | "option" | "datalist") => true,
            Some("optgroup") => in_optgroup,
            _ => false,
        };
        if skip || holds_template_contents::<C>(doc, id) {
            continue;
        }
        let in_optgroup = in_optgroup || tag == Some("optgroup");
        stack.extend(doc.children(id).iter().rev().map(|&child| (child, in_optgroup)));
    }
    options
}

/// Whether `option` is disabled: it has `disabled`, or the optgroup it is in does
/// (<https://html.spec.whatwg.org/#concept-option-disabled>).
pub(crate) fn is_disabled<C: HasDocument>(doc: &C::Document, option: NodeId) -> bool {
    if doc.attribute(option, "disabled").is_some() {
        return true;
    }
    let mut current = option;
    while let Some(ancestor) = doc.parent(current) {
        match html_tag::<C>(doc, ancestor) {
            Some("select" | "hr" | "datalist" | "option") => return false,
            Some("optgroup") => return doc.attribute(ancestor, "disabled").is_some(),
            _ => {}
        }
        current = ancestor;
    }
    false
}

/// The number of rows `select` shows: its `size`, else 4 with `multiple` and 1 without
/// (<https://html.spec.whatwg.org/#concept-select-size>).
pub(crate) fn display_size<C: HasDocument>(doc: &C::Document, select: NodeId) -> u32 {
    match doc.attribute(select, "size").and_then(non_negative_integer) {
        Some(size) => size,
        None if doc.attribute(select, "multiple").is_some() => 4,
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::document_impl::DocumentImpl;
    use crate::parser::Html5Parser;
    use gosub_css3::system::Css3System;
    use gosub_interface::config::ModuleConfiguration;
    use gosub_shared::attributes::Attributes;
    use gosub_shared::byte_stream::Location;

    #[derive(Clone, Debug, PartialEq)]
    struct Config;

    impl ModuleConfiguration for Config {
        type CssSystem = Css3System;
        type Document = DocumentImpl<Self>;
        type HtmlParser = Html5Parser<'static, Self>;
    }

    type Doc = DocumentImpl<Config>;

    fn page(body: &str) -> Doc {
        crate::html_compile::<Config>(&format!("<!DOCTYPE html><body>{body}"))
    }

    fn by_id(doc: &Doc, id: &str) -> NodeId {
        doc.node_by_named_id(id).unwrap_or_else(|| panic!("no #{id}"))
    }

    fn name(doc: &Doc, id: NodeId) -> String {
        doc.attribute(id, "id").unwrap_or_default().to_string()
    }

    /// The ids of the selected options in `select`'s list, in tree order.
    fn selected(doc: &Doc, select: &str) -> Vec<String> {
        list_of_options::<Config>(doc, by_id(doc, select))
            .into_iter()
            .filter(|&o| doc.is_option_selected(o))
            .map(|o| name(doc, o))
            .collect()
    }

    /// A new, unattached HTML element with `attributes`, given as `name=value` pairs.
    fn new_element(doc: &mut Doc, tag: &str, attributes: &[(&str, &str)]) -> NodeId {
        let mut attrs = Attributes::new();
        for (name, value) in attributes {
            attrs.insert((*name).to_string(), (*value).to_string());
        }
        doc.create_element(tag, Some(HTML_NAMESPACE), attrs, Location::default())
    }

    #[test]
    fn disabling_the_selected_option_keeps_it_selected() {
        let cases = [
            // The select fell back to `a`.
            "<select id=s><option id=a><option id=b></select>",
            "<select id=s><option id=a selected><option id=b></select>",
            "<select id=s><optgroup><option id=a></optgroup><option id=b></select>",
        ];
        for body in cases {
            let mut doc = page(body);
            let a = by_id(&doc, "a");
            doc.set_attribute(a, "disabled", "");
            assert_eq!(selected(&doc, "s"), ["a"], "{body}");
        }

        // Nor does disabling the optgroup around it.
        let mut doc = page("<select id=s><optgroup id=g><option id=a></optgroup><option id=b></select>");
        let g = by_id(&doc, "g");
        doc.set_attribute(g, "disabled", "");
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn removing_the_selected_option_selects_the_first_enabled_one() {
        let body = "<select id=s><option id=a selected><option id=b disabled><option id=c></select>";
        let mut doc = page(body);
        let a = by_id(&doc, "a");
        doc.delete_node_by_id(a);
        assert_eq!(selected(&doc, "s"), ["c"]);

        let mut doc = page(body);
        let a = by_id(&doc, "a");
        doc.detach_node(a);
        assert_eq!(selected(&doc, "s"), ["c"]);
        // A removed option keeps its selectedness, and coming back it is the selected one again.
        assert!(doc.is_option_selected(a));
        let select = by_id(&doc, "s");
        doc.attach_node(a, select, None).unwrap();
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn the_selected_attribute_selects_and_deselects() {
        let mut doc = page("<select id=s><option id=a><option id=b><option id=c></select>");
        let (b, c) = (by_id(&doc, "b"), by_id(&doc, "c"));
        doc.set_attribute(b, "selected", "");
        assert_eq!(selected(&doc, "s"), ["b"]);
        // Only adding the attribute counts, not changing its value.
        doc.set_attribute(c, "selected", "");
        doc.set_attribute(b, "selected", "again");
        assert_eq!(selected(&doc, "s"), ["c"]);
        doc.remove_attribute(c, "selected");
        assert_eq!(selected(&doc, "s"), ["a"]);
        // Removing it from an option that is not selected changes nothing.
        doc.remove_attribute(b, "selected");
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn an_inserted_selected_option_deselects_the_one_that_was() {
        for position in [Some(0), Some(1), None] {
            let mut doc = page("<select id=s><option id=a><option id=b selected></select>");
            let select = by_id(&doc, "s");
            let new = new_element(&mut doc, "option", &[("id", "new"), ("selected", "")]);
            doc.attach_node(new, select, position).unwrap();
            assert_eq!(selected(&doc, "s"), ["new"], "at {position:?}");
        }
    }

    #[test]
    fn an_inserted_option_that_is_not_selected_changes_nothing() {
        for position in [Some(0), None] {
            let mut doc = page("<select id=s><option id=a disabled><option id=b></select>");
            let select = by_id(&doc, "s");
            let new = new_element(&mut doc, "option", &[("id", "new")]);
            doc.attach_node(new, select, position).unwrap();
            assert_eq!(selected(&doc, "s"), ["b"], "at {position:?}");
        }
        // With nothing selected yet, it is the first enabled one.
        let mut doc = page("<select id=s><option id=a disabled></select>");
        let select = by_id(&doc, "s");
        assert!(selected(&doc, "s").is_empty());
        let new = new_element(&mut doc, "option", &[("id", "new")]);
        doc.attach_node(new, select, None).unwrap();
        assert_eq!(selected(&doc, "s"), ["new"]);
    }

    #[test]
    fn enabling_an_optgroup_selects_its_first_option() {
        let mut doc =
            page("<select id=s><optgroup id=g disabled><option id=a></optgroup><option id=b disabled></select>");
        assert!(selected(&doc, "s").is_empty());
        let (s, g) = (by_id(&doc, "s"), by_id(&doc, "g"));
        doc.remove_attribute(g, "disabled");
        assert_eq!(selected(&doc, "s"), ["a"]);
        // Disabling it again keeps `a`, and an option joining later does not take over.
        doc.set_attribute(g, "disabled", "");
        let new = new_element(&mut doc, "option", &[("id", "new")]);
        doc.attach_node(new, s, None).unwrap();
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn deleting_a_select_leaves_its_options_selected() {
        for multiple in ["", " multiple"] {
            let body = format!(
                "<select id=s{multiple}><option id=a selected><option id=b selected><option id=c selected></select>"
            );
            let mut doc = page(&body);
            let options = ["a", "b", "c"].map(|id| by_id(&doc, id));
            let before = options.map(|o| doc.is_option_selected(o));
            let s = by_id(&doc, "s");
            doc.delete_node_by_id(s);
            assert_eq!(options.map(|o| doc.is_option_selected(o)), before, "{body}");
        }
    }

    #[test]
    fn moving_an_optgroup_takes_its_selected_option_along() {
        let mut doc = page(
            "<select id=s><option id=a><optgroup id=g><option id=b selected></optgroup></select>\
             <select id=t><option id=c selected></select>",
        );
        let (g, t) = (by_id(&doc, "g"), by_id(&doc, "t"));
        doc.attach_node(g, t, None).unwrap();
        assert_eq!(selected(&doc, "s"), ["a"]);
        assert_eq!(selected(&doc, "t"), ["b"]);
    }

    #[test]
    fn deleting_an_ancestor_takes_its_options_out_of_the_select() {
        let mut doc = page("<select id=s><option id=a><optgroup id=g><option id=b selected></optgroup></select>");
        let (g, b) = (by_id(&doc, "g"), by_id(&doc, "b"));
        doc.delete_node_by_id(g);
        assert_eq!(doc.option_select(b), None);
        assert_eq!(selected(&doc, "s"), ["a"]);

        let mut doc = page("<select id=s><option id=a selected></select>");
        let (s, a) = (by_id(&doc, "s"), by_id(&doc, "a"));
        doc.delete_node_by_id(s);
        assert_eq!(doc.option_select(a), None);
        assert_eq!(doc.first_selected_option(s), None);
    }

    #[test]
    fn a_multiple_select_keeps_every_selected_option() {
        let mut doc = page("<select id=s multiple><option id=a selected><option id=b selected><option id=c></select>");
        assert_eq!(selected(&doc, "s"), ["a", "b"]);
        let s = by_id(&doc, "s");
        assert_eq!(
            doc.first_selected_option(s).map(|o| name(&doc, o)).as_deref(),
            Some("a")
        );
        // Dropping `multiple` keeps the first.
        doc.remove_attribute(s, "multiple");
        assert_eq!(selected(&doc, "s"), ["a"]);

        // No fallback while `multiple` is there, and one as soon as it goes.
        let mut doc = page("<select id=s multiple><option id=a><option id=b></select>");
        assert!(selected(&doc, "s").is_empty());
        let s = by_id(&doc, "s");
        doc.remove_attribute(s, "multiple");
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn only_a_select_showing_one_row_falls_back() {
        let cases = [
            ("<select id=s size=1><option id=a></select>", vec!["a"]),
            ("<select id=s size=x><option id=a></select>", vec!["a"]),
            ("<select id=s size=2><option id=a></select>", vec![]),
            ("<select id=s size=0><option id=a></select>", vec![]),
        ];
        for (body, want) in cases {
            assert_eq!(selected(&page(body), "s"), want, "{body}");
        }
        let mut doc = page("<select id=s size=3><option id=a><option id=b></select>");
        let s = by_id(&doc, "s");
        doc.set_attribute(s, "size", "1");
        assert_eq!(selected(&doc, "s"), ["a"]);
    }

    #[test]
    fn the_list_of_options_follows_the_spec() {
        let mut doc =
            page("<select id=s><option id=a><template><option id=t selected></template><option id=b></select>");
        let s = by_id(&doc, "s");
        // The parser does not put a datalist in a select; the DOM can.
        let datalist = new_element(&mut doc, "datalist", &[]);
        let listed = new_element(&mut doc, "option", &[("id", "d"), ("selected", "")]);
        doc.attach_node(listed, datalist, None).unwrap();
        doc.attach_node(datalist, s, Some(2)).unwrap();
        let options: Vec<_> = list_of_options::<Config>(&doc, s)
            .iter()
            .map(|&o| name(&doc, o))
            .collect();
        assert_eq!(options, ["a", "b"]);
        assert_eq!(selected(&doc, "s"), ["a"]);

        // An option below another element is one of the select's; below a second optgroup it is not.
        let div = new_element(&mut doc, "div", &[]);
        let deep = new_element(&mut doc, "option", &[("id", "deep"), ("selected", "")]);
        doc.attach_node(deep, div, None).unwrap();
        doc.attach_node(div, s, None).unwrap();
        assert_eq!(selected(&doc, "s"), ["deep"]);
        let outer = new_element(&mut doc, "optgroup", &[]);
        let inner = new_element(&mut doc, "optgroup", &[]);
        let nested = new_element(&mut doc, "option", &[("id", "nested"), ("selected", "")]);
        doc.attach_node(nested, inner, None).unwrap();
        doc.attach_node(inner, outer, None).unwrap();
        doc.attach_node(outer, s, None).unwrap();
        assert_eq!(doc.option_select(nested), None);
        assert_eq!(selected(&doc, "s"), ["deep"]);
    }

    #[test]
    fn a_copied_select_has_its_own_selection() {
        let mut doc = page("<select id=s><option id=a><option id=b></select>");
        let (s, b) = (by_id(&doc, "s"), by_id(&doc, "b"));
        doc.set_attribute(b, "selected", "");
        let copy = doc.clone_node(s);
        let options = list_of_options::<Config>(&doc, copy);
        assert_eq!(options.len(), 2);
        assert_eq!(doc.first_selected_option(copy), Some(options[1]));
        // Changing the copy leaves the original alone.
        doc.remove_attribute(options[1], "selected");
        assert_eq!(doc.first_selected_option(copy), Some(options[0]));
        assert_eq!(selected(&doc, "s"), ["b"]);
    }
}
