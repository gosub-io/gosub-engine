/// according to HTML spec:
/// https://html.spec.whatwg.org/#global-attributes
pub(crate) fn is_valid_id_attribute_value(value: &str) -> bool {
    !(value.is_empty() || value.contains(|ref c| char::is_ascii_whitespace(c)))
}
