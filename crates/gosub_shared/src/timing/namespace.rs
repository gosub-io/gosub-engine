//! What the engine measures, and what each measurement means.
//!
//! A timing used to be whatever string a call site typed, which meant a typo produced a new
//! counter that read zero forever, a rename was archaeology, and nothing outside the code
//! could say what a row in a devtools panel actually measured. Naming them here fixes all
//! three, and lets a panel explain itself: the engine is the only thing that knows that
//! `decode.css` is a stylesheet becoming a tree, so the sentence saying so belongs next to
//! the name rather than in whichever shell happens to draw the table.
//!
//! What stays with the shell is presentation -- where the sentence appears, how it is worded
//! for its audience, and translating it. These are definitions, not copy.

/// Where a measurement belongs, for a panel that wants to group rather than list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Bytes over the wire.
    Network,
    /// Bytes becoming a tree.
    Parse,
    /// Deciding what things look like.
    Style,
    /// Deciding where things go.
    Layout,
    /// Putting pixels on the screen.
    Paint,
    /// Answering "what is under the cursor".
    Hover,
    /// Moments in a page's life rather than work done.
    Page,
    /// An embedder's own measurement.
    Embedder,
}

/// Whether a measurement is a span of time or a moment in one.
///
/// The difference matters to anything drawing them: a count, an average and a p95 of a first
/// paint are arithmetic over a single instant, and mean nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A duration: something took this long.
    Span,
    /// A moment: this happened this far into the navigation.
    Mark,
}

/// One thing the engine measures.
///
/// `Other` is the door left open for an embedder timing its own work; everything the engine
/// records itself has a variant, so it can be described, grouped and listed before it has
/// ever fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timing {
    NetDns,
    NetConnect,
    NetTtfb,
    NetFetch,
    NetFetchHtml,
    NetFetchCss,
    NetFetchJs,
    NetFetchImage,
    NetFetchFont,
    NetFetchOther,

    Html5Parse,
    HtmlDocument,
    DecodeHtml,
    DecodeCss,
    DecodeImage,
    ScriptBlockedOnCss,

    PipelineTotal,
    PipelineRenderTree,
    PipelineLayout,
    PipelineLayering,
    PipelineTiling,
    PipelineRasterize,
    PipelinePainting,
    PipelineComposite,
    PipelineExtendTiling,
    PipelineExtendRasterize,
    PipelineExtendPainting,
    PipelineHoverTiling,
    PipelineHoverRasterize,
    PipelineHoverPainting,
    GpuTileRebuild,
    GpuTileComposite,

    HoverTotal,
    HoverHitTest,
    HoverAncestorWalk,
    HoverSetHovered,

    PageDomComplete,
    PageFirstPaint,
    PageLoadComplete,

    /// Something the embedder measures. Named at the call site, and described nowhere,
    /// because the engine does not know what it is.
    Other(&'static str),
}

impl Timing {
    /// The name this is recorded and displayed under.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Timing::NetDns => "net.dns",
            Timing::NetConnect => "net.connect",
            Timing::NetTtfb => "net.ttfb",
            Timing::NetFetch => "net.fetch",
            Timing::NetFetchHtml => "net.fetch.html",
            Timing::NetFetchCss => "net.fetch.css",
            Timing::NetFetchJs => "net.fetch.js",
            Timing::NetFetchImage => "net.fetch.image",
            Timing::NetFetchFont => "net.fetch.font",
            Timing::NetFetchOther => "net.fetch.other",
            Timing::Html5Parse => "html5.parse",
            Timing::HtmlDocument => "html.document",
            Timing::DecodeHtml => "decode.html",
            Timing::DecodeCss => "decode.css",
            Timing::DecodeImage => "decode.image",
            Timing::ScriptBlockedOnCss => "script.blocked_on_css",
            Timing::PipelineTotal => "pipeline.total",
            Timing::PipelineRenderTree => "pipeline.render_tree",
            Timing::PipelineLayout => "pipeline.layout",
            Timing::PipelineLayering => "pipeline.layering",
            Timing::PipelineTiling => "pipeline.tiling",
            Timing::PipelineRasterize => "pipeline.rasterize",
            Timing::PipelinePainting => "pipeline.painting",
            Timing::PipelineComposite => "pipeline.composite",
            Timing::PipelineExtendTiling => "pipeline.extend.tiling",
            Timing::PipelineExtendRasterize => "pipeline.extend.rasterize",
            Timing::PipelineExtendPainting => "pipeline.extend.painting",
            Timing::PipelineHoverTiling => "pipeline.hover.tiling",
            Timing::PipelineHoverRasterize => "pipeline.hover.rasterize",
            Timing::PipelineHoverPainting => "pipeline.hover.painting",
            Timing::GpuTileRebuild => "gputile.rebuild",
            Timing::GpuTileComposite => "gputile.composite",
            Timing::HoverTotal => "hover.total",
            Timing::HoverHitTest => "hover.hit_test",
            Timing::HoverAncestorWalk => "hover.ancestor_walk",
            Timing::HoverSetHovered => "hover.set_hovered",
            Timing::PageDomComplete => "page.dom_complete",
            Timing::PageFirstPaint => "page.first_paint",
            Timing::PageLoadComplete => "page.load_complete",
            Timing::Other(name) => name,
        }
    }

    /// What this measures, in one sentence, for a reader who did not write the engine.
    #[must_use]
    pub const fn describes(&self) -> &'static str {
        match self {
            Timing::NetDns => "Resolving a host name to an address. Only for a request that opened a connection: one served by a pooled connection resolves nothing.",
            Timing::NetConnect => "Opening the connection, TLS handshake included. Encloses the name resolution rather than following it.",
            Timing::NetTtfb => "From sending the request to the first byte of the response: how long the server took to start answering.",
            Timing::NetFetch => "A whole request, from queued to the last byte of its body.",
            Timing::NetFetchHtml => "Fetching a document: the page itself, or one inside a frame.",
            Timing::NetFetchCss => "Fetching a stylesheet. A page waits for these before it can be laid out.",
            Timing::NetFetchJs => "Fetching a script. A blocking one holds up the parse until it arrives.",
            Timing::NetFetchImage => "Fetching an image. These do not block the page, but they are usually most of its bytes.",
            Timing::NetFetchFont => "Fetching a web font a stylesheet declared with @font-face. Text is measured in a fallback face until it lands.",
            Timing::NetFetchOther => "Fetching something the engine has no more specific kind for.",
            Timing::Html5Parse => "Turning HTML into a DOM, tokeniser and tree builder together.",
            Timing::HtmlDocument => "Everything a document needs before a tab can have it: reading the body, parsing it, and putting its stylesheets in place.",
            Timing::DecodeHtml => "Bytes to DOM, and nothing else. Waiting on the network is recorded separately and taken back out of this one.",
            Timing::DecodeCss => "Turning a whole stylesheet's bytes into a tree of rules the cascade can match against.",
            Timing::DecodeImage => "Turning an image's bytes into pixels, decompression and colour conversion included.",
            Timing::ScriptBlockedOnCss => "A script waiting for the stylesheets written before it, which it may not run until they have applied.",
            Timing::PipelineTotal => "One pass of the render pipeline, from document to pixels.",
            Timing::PipelineRenderTree => "Building the render tree: which elements exist, and what they look like.",
            Timing::PipelineLayout => "Deciding where everything goes and how big it is.",
            Timing::PipelineLayering => "Sorting the laid-out boxes into layers that can be drawn independently.",
            Timing::PipelineTiling => "Cutting the page into tiles, so a change can repaint part of it rather than all of it.",
            Timing::PipelineRasterize => "Turning the drawing commands for each tile into actual pixels.",
            Timing::PipelinePainting => "Walking the layers and emitting the drawing commands.",
            Timing::PipelineComposite => "Assembling the finished tiles into the frame.",
            Timing::PipelineExtendTiling => "Tiling again for a page that grew, rather than from scratch.",
            Timing::PipelineExtendRasterize => "Rasterizing the tiles a growing page added.",
            Timing::PipelineExtendPainting => "Painting the tiles a growing page added.",
            Timing::PipelineHoverTiling => "Tiling limited to what a hover changed.",
            Timing::PipelineHoverRasterize => "Rasterizing only the tiles a hover touched.",
            Timing::PipelineHoverPainting => "Painting only what a hover changed.",
            Timing::GpuTileRebuild => "Rebuilding the GPU's copy of a tile after it was repainted.",
            Timing::GpuTileComposite => "Compositing the tiles on the GPU into the frame shown.",
            Timing::HoverTotal => "Everything one mouse position costs: finding what is under it, and updating whatever that changes.",
            Timing::HoverHitTest => "Finding which element is under the cursor, searched through the layers back to front.",
            Timing::HoverAncestorWalk => "Walking up from the element under the cursor, since :hover applies to its ancestors too.",
            Timing::HoverSetHovered => "Applying the change: marking the new chain hovered and the old one not.",
            Timing::PageDomComplete => "When the document finished parsing, measured from the start of the navigation.",
            Timing::PageFirstPaint => "When something first appeared on screen, measured from the start of the navigation.",
            Timing::PageLoadComplete => "When the page and everything it asked for had finished, measured from the start of the navigation.",
            Timing::Other(_) => "Measured by the embedder; the engine does not know what it is.",
        }
    }

    /// Which part of the work this belongs to.
    #[must_use]
    pub const fn group(&self) -> Group {
        match self {
            Timing::NetDns
            | Timing::NetConnect
            | Timing::NetTtfb
            | Timing::NetFetch
            | Timing::NetFetchHtml
            | Timing::NetFetchCss
            | Timing::NetFetchJs
            | Timing::NetFetchImage
            | Timing::NetFetchFont
            | Timing::NetFetchOther => Group::Network,
            Timing::Html5Parse
            | Timing::HtmlDocument
            | Timing::DecodeHtml
            | Timing::DecodeImage
            | Timing::ScriptBlockedOnCss => Group::Parse,
            Timing::DecodeCss | Timing::PipelineRenderTree => Group::Style,
            Timing::PipelineLayout => Group::Layout,
            Timing::PipelineTotal
            | Timing::PipelineLayering
            | Timing::PipelineTiling
            | Timing::PipelineRasterize
            | Timing::PipelinePainting
            | Timing::PipelineComposite
            | Timing::PipelineExtendTiling
            | Timing::PipelineExtendRasterize
            | Timing::PipelineExtendPainting
            | Timing::PipelineHoverTiling
            | Timing::PipelineHoverRasterize
            | Timing::PipelineHoverPainting
            | Timing::GpuTileRebuild
            | Timing::GpuTileComposite => Group::Paint,
            Timing::HoverTotal | Timing::HoverHitTest | Timing::HoverAncestorWalk | Timing::HoverSetHovered => {
                Group::Hover
            }
            Timing::PageDomComplete | Timing::PageFirstPaint | Timing::PageLoadComplete => Group::Page,
            Timing::Other(_) => Group::Embedder,
        }
    }

    /// Whether this is a duration or a moment.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Timing::PageDomComplete | Timing::PageFirstPaint | Timing::PageLoadComplete => Kind::Mark,
            _ => Kind::Span,
        }
    }

    /// The measurement this one happens inside, when it happens inside one.
    ///
    /// Containment is what makes a percentage mean something: without it a reader cannot tell
    /// whether two rows are parts of the same work or two separate pieces of it.
    #[must_use]
    pub const fn parent(&self) -> Option<Timing> {
        match self {
            Timing::PipelineRenderTree
            | Timing::PipelineLayout
            | Timing::PipelineLayering
            | Timing::PipelineTiling
            | Timing::PipelineRasterize
            | Timing::PipelinePainting
            | Timing::PipelineComposite => Some(Timing::PipelineTotal),
            Timing::HoverHitTest | Timing::HoverAncestorWalk | Timing::HoverSetHovered => Some(Timing::HoverTotal),
            Timing::NetDns => Some(Timing::NetConnect),
            Timing::NetConnect | Timing::NetTtfb => Some(Timing::NetFetch),
            Timing::DecodeHtml | Timing::ScriptBlockedOnCss => Some(Timing::HtmlDocument),
            _ => None,
        }
    }

    /// Every measurement the engine knows how to make.
    ///
    /// A panel can list these before any has fired, which is how a stage that is never timed
    /// at all becomes visible -- something a table of what happened cannot show.
    #[must_use]
    pub const fn all() -> &'static [Timing] {
        &[
            Timing::NetDns,
            Timing::NetConnect,
            Timing::NetTtfb,
            Timing::NetFetch,
            Timing::NetFetchHtml,
            Timing::NetFetchCss,
            Timing::NetFetchJs,
            Timing::NetFetchImage,
            Timing::NetFetchFont,
            Timing::NetFetchOther,
            Timing::Html5Parse,
            Timing::HtmlDocument,
            Timing::DecodeHtml,
            Timing::DecodeCss,
            Timing::DecodeImage,
            Timing::ScriptBlockedOnCss,
            Timing::PipelineTotal,
            Timing::PipelineRenderTree,
            Timing::PipelineLayout,
            Timing::PipelineLayering,
            Timing::PipelineTiling,
            Timing::PipelineRasterize,
            Timing::PipelinePainting,
            Timing::PipelineComposite,
            Timing::PipelineExtendTiling,
            Timing::PipelineExtendRasterize,
            Timing::PipelineExtendPainting,
            Timing::PipelineHoverTiling,
            Timing::PipelineHoverRasterize,
            Timing::PipelineHoverPainting,
            Timing::GpuTileRebuild,
            Timing::GpuTileComposite,
            Timing::HoverTotal,
            Timing::HoverHitTest,
            Timing::HoverAncestorWalk,
            Timing::HoverSetHovered,
            Timing::PageDomComplete,
            Timing::PageFirstPaint,
            Timing::PageLoadComplete,
        ]
    }

    /// Recover the measurement a recorded name belongs to.
    ///
    /// `None` for a name the engine does not know, which is either an embedder's own or a
    /// leftover from before it was named here.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Timing> {
        Timing::all().iter().copied().find(|timing| timing.name() == name)
    }
}

impl std::fmt::Display for Timing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Two measurements sharing a name would silently merge in the table, and the reader
    /// would be looking at the sum of two unrelated things.
    #[test]
    fn every_measurement_has_its_own_name() {
        let mut seen = HashSet::new();
        for timing in Timing::all() {
            assert!(seen.insert(timing.name()), "duplicate name: {}", timing.name());
        }
    }

    /// The point of naming them here: a name recorded by the engine can be looked up again,
    /// so a panel showing a row can say what the row means.
    #[test]
    fn a_recorded_name_finds_its_way_back() {
        for timing in Timing::all() {
            assert_eq!(Timing::from_name(timing.name()), Some(*timing));
        }
        assert_eq!(Timing::from_name("embedder.something"), None);
    }

    /// A description that says nothing is worse than none: it looks like an answer.
    #[test]
    fn every_measurement_says_what_it_measures() {
        for timing in Timing::all() {
            let description = timing.describes();
            assert!(description.len() > 20, "{} is barely described", timing.name());
            assert!(description.ends_with('.'), "{} reads as a fragment", timing.name());
        }
    }

    /// A parent that is not itself measured would leave a child hanging off nothing.
    #[test]
    fn every_parent_is_itself_a_measurement() {
        for timing in Timing::all() {
            if let Some(parent) = timing.parent() {
                assert!(
                    Timing::all().contains(&parent),
                    "{} hangs off {}, which is not measured",
                    timing.name(),
                    parent.name()
                );
                assert_ne!(parent, *timing, "{} contains itself", timing.name());
            }
        }
    }
}
