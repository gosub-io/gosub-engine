use gosub_html5::async_parse::{HintKind, ResourceHint as HtmlResourceHint};
use gosub_net::decision::types::RequestDestination;
use gosub_net::net_types::{Priority, ResourceKind};
use url::Url;

/// Convert a `gosub_html5` resource hint to `gosub_net` fetch request components.
pub fn hint_to_net(hint: HtmlResourceHint) -> (Url, ResourceKind, RequestDestination, Priority) {
    let (kind, dest, priority) = match hint.kind {
        HintKind::Stylesheet => (ResourceKind::Stylesheet, RequestDestination::Style, Priority::High),
        HintKind::Script { .. } => (
            ResourceKind::Script { blocking: false },
            RequestDestination::Script,
            Priority::Normal,
        ),
        HintKind::Image => (ResourceKind::Image, RequestDestination::Image, Priority::Low),
        HintKind::Font => (ResourceKind::Font, RequestDestination::Font, Priority::Normal),
    };
    (hint.url, kind, dest, priority)
}

