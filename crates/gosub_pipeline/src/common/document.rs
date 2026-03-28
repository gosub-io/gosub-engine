pub mod node;
pub mod style;
pub mod document;
pub mod parser;

#[cfg(test)]
pub fn create_document() -> document::Document {
    use node::AttrMap;

    let mut doc = document::Document::new("about:blank");

    let mut html_attrs = AttrMap::new();
    html_attrs.set("lang", "en");
    let html_id = doc.new_element(None, "html", Some(html_attrs), false, None);
    doc.set_root(html_id);

    let body_id = doc.new_element(Some(html_id), "body", None, false, None);

    let mut h1_attrs = AttrMap::new();
    h1_attrs.set("class", "title");
    h1_attrs.set("data-alpine", "x-wrap");
    let h1_id = doc.new_element(Some(body_id), "h1", Some(h1_attrs), false, None);
    doc.new_text(Some(h1_id), "header", None);

    let mut script_attrs = AttrMap::new();
    script_attrs.set("async", "true");
    script_attrs.set("src", "script.js");
    script_attrs.set("type", "text/javascript");
    doc.new_element(Some(body_id), "script", Some(script_attrs), false, None);

    let mut p_attrs = AttrMap::new();
    p_attrs.set("class", "paragraph");
    let p_id = doc.new_element(Some(body_id), "p", Some(p_attrs), false, None);

    let strong_id = doc.new_element(Some(p_id), "strong", None, false, None);
    doc.new_text(Some(strong_id), "strong", None);

    let mut img_attrs = AttrMap::new();
    img_attrs.set("src", "image.jpg");
    img_attrs.set("alt", "image");
    doc.new_element(Some(p_id), "img", Some(img_attrs), true, None);

    doc
}