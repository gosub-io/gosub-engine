use gosub_interface::node::ShadowRootInit;
use gosub_shared::node::NodeId;

/// Data for a shadow root node - the root of a shadow tree.
///
/// The node lives in the same arena as the light DOM and owns its children normally; what makes
/// it a shadow root is that its host reaches it through `ElementData::shadow_root` rather than
/// through `children`, and that `parent` is `None` so no ancestor walk can escape the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowRootData {
    /// The element this shadow tree hangs off. Always a valid, live node.
    pub host: NodeId,
    /// The flags the root was attached with.
    pub init: ShadowRootInit,
}

impl ShadowRootData {
    #[must_use]
    pub fn new(host: NodeId, init: ShadowRootInit) -> Self {
        Self { host, init }
    }
}
