use derive_more::Display;

/// A `NodeID` is a unique identifier for a node in a node tree.
#[derive(Clone, Copy, Debug, Default, Display, Eq, Hash, PartialEq, PartialOrd)]
pub struct NodeId(usize);

impl From<NodeId> for usize {
    /// Converts a `NodeId` into a usize
    fn from(value: NodeId) -> Self {
        value.0
    }
}

impl From<usize> for NodeId {
    /// Converts a usize into a `NodeId`
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<u64> for NodeId {
    /// Converts a u64 into a `NodeId`
    fn from(value: u64) -> Self {
        Self(value as usize)
    }
}

impl From<NodeId> for u64 {
    /// Converts a `NodeId` into a u64
    fn from(value: NodeId) -> Self {
        value.0 as u64
    }
}

impl Default for &NodeId {
    /// Returns the default `NodeId`, which is 0
    fn default() -> Self {
        &NodeId(0)
    }
}

impl NodeId {
    // TODO: Drop Default derive and only use 0 for the root, or choose another id for the root
    pub const ROOT_NODE: usize = 0;

    /// Returns the root node ID
    #[must_use]
    pub fn root() -> Self {
        Self(Self::ROOT_NODE)
    }

    /// Returns true when this nodeId is the root node
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == Self::ROOT_NODE
    }

    /// Returns the next node ID
    #[must_use]
    pub fn next(&self) -> Self {
        if self.0 == usize::MAX {
            return Self(usize::MAX);
        }

        Self(self.0 + 1)
    }

    /// Returns the nodeID as usize
    #[must_use]
    pub fn as_usize(&self) -> usize {
        self.0
    }

    /// Returns the previous node ID
    #[must_use]
    pub fn prev(&self) -> Self {
        if self.0 == 0 {
            return Self::root();
        }

        Self(self.0 - 1)
    }
}
