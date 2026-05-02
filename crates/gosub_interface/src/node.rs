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
}
