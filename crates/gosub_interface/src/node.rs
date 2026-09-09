#[derive(PartialEq, Debug, Copy, Clone)]
pub enum QuirksMode {
    Quirks,
    LimitedQuirks,
    NoQuirks,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum NodeType {
    DocumentNode,
    DocTypeNode,
    TextNode,
    CommentNode,
    ElementNode,
    /// Root of a shadow tree. A `DocumentFragment` in the spec, but a kind of its own here
    /// because it is the only node that hangs off its parent by a side pointer rather than by
    /// appearing in the parent's `children`.
    ShadowRootNode,
}

/// Encapsulation mode of a shadow root. Stored but not enforced: `Closed` differs from `Open`
/// only in what script may reach, and there is no scripting yet.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum ShadowRootMode {
    Open,
    Closed,
}

impl ShadowRootMode {
    /// Parses the `shadowrootmode` content attribute. `None` is the attribute's "none" state:
    /// absent, or any value that is not an ASCII case-insensitive `open` or `closed`.
    #[must_use]
    pub fn from_attribute(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("open") {
            Some(Self::Open)
        } else if value.eq_ignore_ascii_case("closed") {
            Some(Self::Closed)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_attribute(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

/// How slottables are assigned to slots. Declarative shadow roots are always `Named`; `Manual`
/// is only reachable through `attachShadow()`, which does not exist yet.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum SlotAssignmentMode {
    Named,
    Manual,
}

/// Everything a shadow root carries besides its host and its children - the arguments to the
/// spec's "attach a shadow root".
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct ShadowRootInit {
    pub mode: ShadowRootMode,
    pub delegates_focus: bool,
    pub clonable: bool,
    pub serializable: bool,
    pub slot_assignment: SlotAssignmentMode,
}

impl ShadowRootInit {
    /// `mode` with the default every other flag takes when its attribute is absent.
    #[must_use]
    pub fn new(mode: ShadowRootMode) -> Self {
        Self {
            mode,
            delegates_focus: false,
            clonable: false,
            serializable: false,
            slot_assignment: SlotAssignmentMode::Named,
        }
    }
}

/// Elements that may host a shadow tree, besides valid custom element names.
/// <https://dom.spec.whatwg.org/#valid-shadow-host-name>
const VALID_SHADOW_HOST_NAMES: [&str; 18] = [
    "article",
    "aside",
    "blockquote",
    "body",
    "div",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "span",
];

/// Hyphenated names that look like custom elements but are reserved by
/// `PotentialCustomElementName` for legacy SVG and MathML elements.
///
/// These *are* reachable in the HTML namespace: the HTML parser only puts `<font-face>` in the
/// SVG namespace inside an `<svg>` subtree, so the same tag in ordinary markup produces an
/// HTML-namespace element with this local name. Accepting one would let `attachShadow` succeed
/// on an element that is not a valid custom element.
const RESERVED_HYPHENATED_NAMES: [&str; 8] = [
    "annotation-xml",
    "color-profile",
    "font-face",
    "font-face-format",
    "font-face-name",
    "font-face-src",
    "font-face-uri",
    "missing-glyph",
];

/// Whether `name` (an HTML-namespace local name) may host a shadow tree.
///
/// Either one of the built-in elements the spec allows, or a valid custom element name: led by
/// a lowercase ASCII letter, containing a hyphen, carrying no ASCII uppercase, and not one of
/// the reserved names above.
#[must_use]
pub fn is_valid_shadow_host_name(name: &str) -> bool {
    if VALID_SHADOW_HOST_NAMES.contains(&name) {
        return true;
    }
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.contains('-')
        // `my-Widget` is not a custom element name. HTML parsing lowercases tag names, so this
        // is unreachable from markup, but the check is cheap and the function is public.
        && !name.contains(|c: char| c.is_ascii_uppercase())
        && !RESERVED_HYPHENATED_NAMES.contains(&name)
}

#[cfg(test)]
mod shadow_host_name_tests {
    use super::is_valid_shadow_host_name;

    #[test]
    fn accepts_built_ins_and_custom_element_names() {
        assert!(is_valid_shadow_host_name("div"));
        assert!(is_valid_shadow_host_name("span"));
        assert!(is_valid_shadow_host_name("my-widget"));
        assert!(is_valid_shadow_host_name("x-"));
    }

    #[test]
    fn rejects_names_that_are_not_custom_elements() {
        assert!(!is_valid_shadow_host_name("table"), "not a permitted built-in host");
        assert!(!is_valid_shadow_host_name("widget"), "no hyphen");
        assert!(!is_valid_shadow_host_name("-widget"), "does not start with a letter");
        assert!(!is_valid_shadow_host_name("1-widget"), "does not start with a letter");
    }

    #[test]
    fn rejects_uppercase_and_reserved_names() {
        assert!(!is_valid_shadow_host_name("my-Widget"));
        assert!(!is_valid_shadow_host_name("My-widget"));
        for reserved in ["font-face", "annotation-xml", "missing-glyph", "color-profile"] {
            assert!(!is_valid_shadow_host_name(reserved), "{reserved} is reserved");
        }
    }
}
