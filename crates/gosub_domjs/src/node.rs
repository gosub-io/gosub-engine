//! The JS `Node` wrapper.
//!
//! One class covers every node type and dispatches the element-specific properties on tag
//! name. A real binding needs the interface hierarchy (`HTMLOptionElement` and friends) so
//! that `instanceof` and prototype-chain tests work; this is enough to drive the engine.

use cow_utils::CowUtils;
use gosub_interface::document::Document as _;
use gosub_interface::node::NodeType;
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use rquickjs::class::Trace;
use rquickjs::{Ctx, Exception, JsLifetime, Result, Value};

use crate::{event, select, text, wrap, wrap_opt, DocHandle};

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Node")]
pub struct GosubNode {
    #[qjs(skip_trace)]
    doc: DocHandle,
    #[qjs(skip_trace)]
    pub(crate) id: NodeId,
}

impl GosubNode {
    pub(crate) fn new(doc: DocHandle, id: NodeId) -> Self {
        Self { doc, id }
    }

    /// The document has no namespace support for attributes, so a namespaced attribute is
    /// parked under a key no HTML attribute name can produce (they cannot contain spaces).
    /// That keeps `setAttributeNS` out of the reflection path, which is what the tests check.
    fn ns_key(namespace: &str, name: &str) -> String {
        format!("{namespace} {name}")
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl GosubNode {
    // ── node ───────────────────────────────────────────────────────────────

    #[qjs(get)]
    pub fn node_type(&self) -> u32 {
        match self.doc.borrow().node_type(self.id) {
            NodeType::ElementNode => 1,
            NodeType::TextNode => 3,
            NodeType::CommentNode => 8,
            NodeType::DocumentNode => 9,
            NodeType::DocTypeNode => 10,
            // A shadow root is a DocumentFragment as far as the DOM API is concerned.
            NodeType::ShadowRootNode => 11,
        }
    }

    #[qjs(get)]
    pub fn node_name(&self) -> String {
        let doc = self.doc.borrow();
        match doc.tag_name(self.id) {
            Some(tag) => tag.cow_to_ascii_uppercase().into_owned(),
            None => match doc.node_type(self.id) {
                NodeType::TextNode => "#text".to_string(),
                NodeType::CommentNode => "#comment".to_string(),
                NodeType::ShadowRootNode => "#document-fragment".to_string(),
                _ => "#document".to_string(),
            },
        }
    }

    #[qjs(get)]
    pub fn tag_name(&self) -> Option<String> {
        self.doc
            .borrow()
            .tag_name(self.id)
            .map(|tag| tag.cow_to_ascii_uppercase().into_owned())
    }

    #[qjs(get)]
    pub fn local_name(&self) -> Option<String> {
        self.doc.borrow().tag_name(self.id).map(str::to_string)
    }

    // ── tree ───────────────────────────────────────────────────────────────

    #[qjs(get)]
    pub fn parent_node<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let parent = self.doc.borrow().parent(self.id);
        wrap_opt(&ctx, &self.doc, parent)
    }

    #[qjs(get)]
    pub fn parent_element<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let parent = self
            .doc
            .borrow()
            .parent(self.id)
            .filter(|&p| self.doc.borrow().tag_name(p).is_some());
        wrap_opt(&ctx, &self.doc, parent)
    }

    #[qjs(get)]
    pub fn child_nodes<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let children = self.doc.borrow().children(self.id).to_vec();
        wrap_list(&ctx, &self.doc, &children)
    }

    #[qjs(get)]
    pub fn children<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let doc = self.doc.borrow();
        let children: Vec<NodeId> = doc
            .children(self.id)
            .iter()
            .copied()
            .filter(|&c| doc.tag_name(c).is_some())
            .collect();
        drop(doc);
        wrap_list(&ctx, &self.doc, &children)
    }

    #[qjs(get)]
    pub fn first_child<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let first = self.doc.borrow().children(self.id).first().copied();
        wrap_opt(&ctx, &self.doc, first)
    }

    pub fn append_child<'js>(&self, ctx: Ctx<'js>, child: rquickjs::Class<'js, GosubNode>) -> Result<Value<'js>> {
        let child_id = child.borrow().id;
        {
            let mut doc = self.doc.borrow_mut();
            doc.detach(child_id);
            doc.attach(child_id, self.id, None);
        }
        if self.doc.borrow().parent(child_id) != Some(self.id) {
            // `attach_node` refuses to build a cycle instead of throwing HierarchyRequestError.
            return Err(Exception::throw_message(&ctx, "appendChild would create a cycle"));
        }
        wrap(&ctx, &self.doc, child_id)
    }

    pub fn remove_child<'js>(&self, ctx: Ctx<'js>, child: rquickjs::Class<'js, GosubNode>) -> Result<Value<'js>> {
        let child_id = child.borrow().id;
        if self.doc.borrow().parent(child_id) != Some(self.id) {
            return Err(Exception::throw_message(&ctx, "NotFoundError: node is not a child"));
        }
        self.doc.borrow_mut().detach(child_id);
        wrap(&ctx, &self.doc, child_id)
    }

    pub fn remove(&self) {
        self.doc.borrow_mut().detach(self.id);
    }

    pub fn has_child_nodes(&self) -> bool {
        !self.doc.borrow().children(self.id).is_empty()
    }

    // ── attributes ─────────────────────────────────────────────────────────

    pub fn get_attribute(&self, name: String) -> Option<String> {
        self.doc
            .borrow()
            .attribute(self.id, &name.cow_to_ascii_lowercase())
            .map(str::to_string)
    }

    pub fn set_attribute(&self, name: String, value: String) {
        self.doc
            .borrow_mut()
            .set_attribute(self.id, &name.cow_to_ascii_lowercase(), &value);
    }

    pub fn remove_attribute(&self, name: String) {
        self.doc
            .borrow_mut()
            .remove_attribute(self.id, &name.cow_to_ascii_lowercase());
    }

    pub fn has_attribute(&self, name: String) -> bool {
        self.doc
            .borrow()
            .attribute(self.id, &name.cow_to_ascii_lowercase())
            .is_some()
    }

    #[qjs(rename = "setAttributeNS")]
    pub fn set_attribute_ns(&self, namespace: Option<String>, name: String, value: String) {
        match namespace {
            None => self.set_attribute(name, value),
            Some(ns) => self
                .doc
                .borrow_mut()
                .set_attribute(self.id, &Self::ns_key(&ns, &name), &value),
        }
    }

    #[qjs(rename = "getAttributeNS")]
    pub fn get_attribute_ns(&self, namespace: Option<String>, name: String) -> Option<String> {
        match namespace {
            None => self.get_attribute(name),
            Some(ns) => self
                .doc
                .borrow()
                .attribute(self.id, &Self::ns_key(&ns, &name))
                .map(str::to_string),
        }
    }

    // ── reflected content attributes ───────────────────────────────────────

    #[qjs(get)]
    pub fn id(&self) -> String {
        self.get_attribute("id".into()).unwrap_or_default()
    }

    #[qjs(set, rename = "id")]
    pub fn set_id(&self, value: String) {
        self.set_attribute("id".into(), value);
    }

    #[qjs(get)]
    pub fn class_name(&self) -> String {
        self.get_attribute("class".into()).unwrap_or_default()
    }

    #[qjs(set, rename = "className")]
    pub fn set_class_name(&self, value: String) {
        self.set_attribute("class".into(), value);
    }

    /// `HTMLInputElement.type` / `HTMLTextAreaElement.type`.
    #[qjs(get, rename = "type")]
    pub fn control_type(&self) -> Option<String> {
        let doc = self.doc.borrow();
        match doc.tag_name(self.id) {
            Some("textarea") => Some("textarea".to_string()),
            Some("input") => Some(
                doc.attribute(self.id, "type")
                    .map(|t| t.cow_to_ascii_lowercase().into_owned())
                    .unwrap_or_else(|| "text".to_string()),
            ),
            _ => None,
        }
    }

    /// `HTMLOptionElement.text`: the option's child text, stripped and collapsed.
    #[qjs(get)]
    pub fn text(&self) -> Option<String> {
        let doc = self.doc.borrow();
        if doc.tag_name(self.id) != Some("option") {
            return None;
        }
        Some(text::strip_and_collapse(&text::descendant_text(&doc, self.id)))
    }

    /// `HTMLOptionElement.value`: the `value` attribute if present, else the option's text.
    #[qjs(get)]
    pub fn value(&self) -> Option<String> {
        let doc = self.doc.borrow();
        if doc.tag_name(self.id) != Some("option") {
            return None;
        }
        if let Some(value) = doc.attribute(self.id, "value") {
            return Some(value.to_string());
        }
        drop(doc);
        self.text()
    }

    /// `HTMLOptionElement.label`: the `label` attribute if present, else the option's text.
    #[qjs(get)]
    pub fn label(&self) -> Option<String> {
        let doc = self.doc.borrow();
        if doc.tag_name(self.id) != Some("option") {
            return None;
        }
        if let Some(label) = doc.attribute(self.id, "label") {
            return Some(label.to_string());
        }
        drop(doc);
        self.text()
    }

    // ── content ────────────────────────────────────────────────────────────

    #[qjs(get)]
    pub fn text_content(&self) -> String {
        let doc = self.doc.borrow();
        text::descendant_text(&doc, self.id)
    }

    #[qjs(set, rename = "textContent")]
    pub fn set_text_content(&self, value: String) {
        let children = self.doc.borrow().children(self.id).to_vec();
        let mut doc = self.doc.borrow_mut();
        for child in children {
            // detach, not remove: `remove` deletes the node out of the arena, and a test that
            // still holds a wrapper for a child would be left pointing at a freed id. This is
            // the same thing removeChild does.
            doc.detach(child);
        }
        if !value.is_empty() {
            let text = doc.create_text(&value, Location::default());
            doc.attach(text, self.id, None);
        }
    }

    #[qjs(get, rename = "outerHTML")]
    pub fn outer_html(&self) -> String {
        self.doc.borrow().write_from_node(self.id)
    }

    // ── queries ────────────────────────────────────────────────────────────

    pub fn query_selector<'js>(&self, ctx: Ctx<'js>, selector: String) -> Result<Value<'js>> {
        let found = crate::document::first_match(&self.doc.borrow(), self.id, &selector)
            .map_err(|e| Exception::throw_message(&ctx, &e))?;
        wrap_opt(&ctx, &self.doc, found)
    }

    #[qjs(rename = "getElementsByTagName")]
    pub fn get_elements_by_tag_name<'js>(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        let doc = self.doc.borrow();
        let found: Vec<NodeId> = select::descendants(&doc, self.id)
            .into_iter()
            // "*" is the wildcard: every element, not an element whose tag is literally "*".
            .filter(|&id| name == "*" || doc.tag_name(id).is_some_and(|tag| tag.eq_ignore_ascii_case(&name)))
            .collect();
        drop(doc);
        wrap_list(&ctx, &self.doc, &found)
    }

    // ── events ─────────────────────────────────────────────────────────────

    #[qjs(rename = "addEventListener")]
    pub fn add_event_listener<'js>(
        &self,
        ctx: Ctx<'js>,
        event_type: String,
        callback: rquickjs::Function<'js>,
        options: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<()> {
        event::add(&ctx, u64::from(self.id), event_type, callback, options)
    }

    #[qjs(rename = "removeEventListener")]
    pub fn remove_event_listener<'js>(
        &self,
        ctx: Ctx<'js>,
        event_type: String,
        callback: rquickjs::Function<'js>,
        options: rquickjs::prelude::Opt<Value<'js>>,
    ) -> Result<()> {
        event::remove(&ctx, u64::from(self.id), &event_type, &callback, options)
    }

    #[qjs(rename = "dispatchEvent")]
    pub fn dispatch_event<'js>(
        &self,
        ctx: Ctx<'js>,
        event: rquickjs::Class<'js, event::DomEvent<'js>>,
    ) -> Result<bool> {
        event::dispatch(&ctx, &self.doc, self.id, event)
    }

    /// Fires a click event. There is no activation behaviour behind it yet - a checkbox does
    /// not toggle and a submit button does not submit, because those live in engine code
    /// this crate cannot reach.
    pub fn click<'js>(&self, ctx: Ctx<'js>) -> Result<()> {
        let event = rquickjs::Class::instance(ctx.clone(), event::DomEvent::synthetic("click", true, true))?;
        event::dispatch(&ctx, &self.doc, self.id, event)?;
        Ok(())
    }

    #[qjs(rename = "toString")]
    pub fn to_string_js(&self) -> String {
        format!("[object Node {}]", self.node_name())
    }
}

/// A plain JS array; a real `NodeList`/`HTMLCollection` is live and has `item()`.
pub(crate) fn wrap_list<'js>(ctx: &Ctx<'js>, doc: &DocHandle, ids: &[NodeId]) -> Result<Value<'js>> {
    let array = rquickjs::Array::new(ctx.clone())?;
    for (index, &id) in ids.iter().enumerate() {
        array.set(index, wrap(ctx, doc, id)?)?;
    }
    Ok(array.into_value())
}
