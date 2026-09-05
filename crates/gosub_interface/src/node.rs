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

/// Whether `name` (an HTML-namespace local name) may host a shadow tree.
///
/// Custom element names are accepted by shape - a name led by a lowercase ASCII letter and
/// containing a hyphen - rather than by the full `PotentialCustomElementName` grammar, which
/// additionally excludes a handful of legacy hyphenated SVG and MathML names. Those never
/// appear in the HTML namespace, so the difference is not observable here.
#[must_use]
pub fn is_valid_shadow_host_name(name: &str) -> bool {
    if VALID_SHADOW_HOST_NAMES.contains(&name) {
        return true;
    }
    name.starts_with(|c: char| c.is_ascii_lowercase()) && name.contains('-')
}
