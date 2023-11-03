/// according to HTML5 spec: 3.2.3.1
/// https://www.w3.org/TR/2011/WD-html5-20110405/elements.html#the-id-attribute
pub(crate) fn is_valid_id_attribute_value(value: &str) -> bool {
    if value.contains(char::is_whitespace) {
        return false;
    }

    if value.is_empty() {
        return false;
    }

    // must contain at least one character,
    // but doesn't specify it should *start* with a character
    value.contains(char::is_alphabetic)
}
