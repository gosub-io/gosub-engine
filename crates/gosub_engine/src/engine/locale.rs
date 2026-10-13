//! The `Accept-Language` a zone sends when its configuration names none, taken from the
//! languages the user set for the operating system.
//!
//! Fetch leaves the value to the user agent (§4.1, "user agents should append ...
//! `Accept-Language`"), and gosub-sonar sends none of its own, so without this a page never
//! learned which language to answer in.

use cow_utils::CowUtils;
use std::sync::OnceLock;

/// Lowest weight a language gets, however far down the list it is.
const MIN_Q: u32 = 1;

/// The OS languages as an `Accept-Language` value: `None` when the system names none, or
/// only the `C`/`POSIX` locale, which is no language at all. Read once: a running process
/// does not see the locale change.
pub(crate) fn os_accept_language() -> Option<String> {
    static VALUE: OnceLock<Option<String>> = OnceLock::new();
    VALUE
        .get_or_init(|| accept_language_for(sys_locale::get_locales()))
        .clone()
}

/// An `Accept-Language` value for `locales`, BCP 47 tags in the user's order of preference.
///
/// Each tag with a region is followed by its language alone (`en-US` then `en`), as browsers
/// send it, so a server that only has `en` still matches. Weights fall by 0.1 from the
/// second entry on, down to 0.1 (RFC 9110 §12.5.4). A tag that is not plain letters, digits
/// and hyphens, or that names the `C`/`POSIX` locale, is left out.
pub(crate) fn accept_language_for(locales: impl IntoIterator<Item = String>) -> Option<String> {
    let mut tags: Vec<String> = Vec::new();
    for locale in locales {
        let tag = locale.trim();
        if tag.is_empty()
            || tag.eq_ignore_ascii_case("c")
            || tag.eq_ignore_ascii_case("posix")
            || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            continue;
        }
        let language = tag
            .split('-')
            .next()
            .unwrap_or(tag)
            .cow_to_ascii_lowercase()
            .into_owned();
        for candidate in [tag.to_string(), language] {
            if !tags.iter().any(|t| t.eq_ignore_ascii_case(&candidate)) {
                tags.push(candidate);
            }
        }
    }
    if tags.is_empty() {
        return None;
    }
    let value = tags
        .iter()
        .enumerate()
        .map(|(i, tag)| match 10u32.saturating_sub(i as u32).max(MIN_Q) {
            10 => tag.clone(),
            q => format!("{tag};q=0.{q}"),
        })
        .collect::<Vec<_>>()
        .join(",");
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(locales: &[&str]) -> Option<String> {
        accept_language_for(locales.iter().map(|l| l.to_string()))
    }

    #[test]
    fn a_region_is_followed_by_its_language() {
        assert_eq!(value(&["en-US"]).as_deref(), Some("en-US,en;q=0.9"));
        assert_eq!(value(&["nl"]).as_deref(), Some("nl"));
    }

    #[test]
    fn languages_keep_the_users_order_with_falling_weights() {
        assert_eq!(
            value(&["nl-NL", "en-US", "en"]).as_deref(),
            Some("nl-NL,nl;q=0.9,en-US;q=0.8,en;q=0.7")
        );
    }

    #[test]
    fn weights_stop_at_a_tenth() {
        let many: Vec<String> = (0..12).map(|i| format!("x{i}")).collect();
        let v = accept_language_for(many).unwrap();
        assert!(v.ends_with("x11;q=0.1"), "{v}");
        assert!(!v.contains("q=0.0"), "{v}");
    }

    #[test]
    fn no_language_means_no_header() {
        assert_eq!(value(&[]), None);
        assert_eq!(value(&["C"]), None);
        assert_eq!(value(&["POSIX", ""]), None);
        assert_eq!(value(&["en US", "fr;q=1"]), None);
        assert_eq!(value(&["C", "de-DE"]).as_deref(), Some("de-DE,de;q=0.9"));
    }
}
