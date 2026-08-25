//! The JS `document` object.

use cow_utils::CowUtils;
use gosub_interface::document::Document as _;
use gosub_shared::byte_stream::Location;
use gosub_shared::node::NodeId;
use rquickjs::class::Trace;
use rquickjs::{Ctx, Exception, JsLifetime, Result, Value};
use std::collections::HashMap;

use crate::node::wrap_list;
use crate::{select, wrap, wrap_opt, Doc, DocHandle};

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Document")]
pub struct GosubDocument {
    #[qjs(skip_trace)]
    doc: DocHandle,
}

impl GosubDocument {
    pub(crate) fn new(doc: DocHandle) -> Self {
        Self { doc }
    }

    fn first_tag(&self, name: &str) -> Option<NodeId> {
        let doc = self.doc.borrow();
        let root = doc.root();
        select::descendants(&doc, root)
            .into_iter()
            .find(|&id| doc.tag_name(id).is_some_and(|tag| tag.eq_ignore_ascii_case(name)))
    }
}

#[rquickjs::methods(rename_all = "camelCase")]
impl GosubDocument {
    #[qjs(rename = "getElementById")]
    pub fn get_element_by_id<'js>(&self, ctx: Ctx<'js>, id: String) -> Result<Value<'js>> {
        let found = self.doc.borrow().node_by_named_id(&id);
        wrap_opt(&ctx, &self.doc, found)
    }

    pub fn create_element<'js>(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        let id = self.doc.borrow_mut().create_element(
            &name.cow_to_ascii_lowercase(),
            Some(gosub_html5::node::HTML_NAMESPACE),
            HashMap::new(),
            Location::default(),
        );
        wrap(&ctx, &self.doc, id)
    }

    pub fn create_text_node<'js>(&self, ctx: Ctx<'js>, data: String) -> Result<Value<'js>> {
        let id = self.doc.borrow_mut().create_text(&data, Location::default());
        wrap(&ctx, &self.doc, id)
    }

    pub fn query_selector<'js>(&self, ctx: Ctx<'js>, selector: String) -> Result<Value<'js>> {
        let doc = self.doc.borrow();
        let root = doc.root();
        let found = first_match(&doc, root, &selector).map_err(|e| Exception::throw_message(&ctx, &e))?;
        drop(doc);
        wrap_opt(&ctx, &self.doc, found)
    }

    #[qjs(rename = "getElementsByTagName")]
    pub fn get_elements_by_tag_name<'js>(&self, ctx: Ctx<'js>, name: String) -> Result<Value<'js>> {
        let doc = self.doc.borrow();
        let root = doc.root();
        let found: Vec<NodeId> = select::descendants(&doc, root)
            .into_iter()
            .filter(|&id| doc.tag_name(id).is_some_and(|tag| tag.eq_ignore_ascii_case(&name)))
            .collect();
        drop(doc);
        wrap_list(&ctx, &self.doc, &found)
    }

    #[qjs(get)]
    pub fn body<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let found = self.first_tag("body");
        wrap_opt(&ctx, &self.doc, found)
    }

    #[qjs(get)]
    pub fn head<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let found = self.first_tag("head");
        wrap_opt(&ctx, &self.doc, found)
    }

    #[qjs(get)]
    pub fn document_element<'js>(&self, ctx: Ctx<'js>) -> Result<Value<'js>> {
        let found = self.first_tag("html");
        wrap_opt(&ctx, &self.doc, found)
    }
}

/// First element descendant of `root` matching `selector`, in tree order.
pub(crate) fn first_match(doc: &Doc, root: NodeId, selector: &str) -> std::result::Result<Option<NodeId>, String> {
    let compound = select::parse(selector)?;
    Ok(select::descendants(doc, root)
        .into_iter()
        .find(|&id| select::matches(doc, id, &compound)))
}
