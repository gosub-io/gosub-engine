use std::collections::HashMap;

pub struct ShadowRoot {
    pub mode: ShadowRootMode,
    pub delegates_focus: bool,
    pub slot_assignment: SlotAssignmentMode,
    pub host: Box<Element>,
    // pub onslotchange: Option<EventHandler>,
}

pub enum SlotAssignmentMode {
    Manual,
    Named,
}

pub enum ShadowRootMode {
    Open,
    Closed,
}

pub struct Element {
    pub namespace_uri: Option<String>,
    pub prefix: Option<String>,
    pub local_name: String,
    pub tag_name: String,
    pub id: String,
    pub class_name: String,
    pub class_list: Vec<String>,
    pub slot: String,
    pub attributes: HashMap<String, String>,
    pub shadow_root: Option<Box<ShadowRoot>>,
}

pub struct HtmlElement {
    // Element fields
    pub namespace_uri: Option<String>,
    pub prefix: Option<String>,
    pub local_name: String,
    pub tag_name: String,
    pub id: String,
    pub class_name: String,
    pub class_list: Vec<String>,
    pub slot: String,
    pub attributes: HashMap<String, String>,
    pub shadow_root: Option<ShadowRoot>,

    // HTML Element
    pub title: String,
    pub lang: String,
    pub translate: bool,
    pub dir: String,

    pub hidden: Option<bool>,
    pub insert: bool,
    pub access_key: String,
    pub access_key_label: String,
    pub draggable: bool,
    pub spellcheck: bool,
    pub autocapitalize: String,

    pub inner_text: String,
    pub outer_text: String,

    pub popover: Option<String>,
}
