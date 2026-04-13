use crate::decision_hub::DecisionToken;
use crate::net_types::{FetchHandle, FetchRequest, FetchResult};
use crate::types::{Action, ZoneId};

pub type IoChannel = tokio::sync::mpsc::UnboundedSender<IoCommand>;

pub enum IoCommand {
    Fetch {
        zone_id: ZoneId,
        req: Box<FetchRequest>,
        handle: FetchHandle,
        reply_tx: tokio::sync::oneshot::Sender<FetchResult>,
    },
    Decision {
        zone_id: ZoneId,
        token: DecisionToken,
        action: Action,
    },
    ShutdownZone {
        zone_id: ZoneId,
        reply_tx: tokio::sync::oneshot::Sender<()>,
    },
}
