//! The engine's message vocabulary: commands flowing in ([`TabCommand`], [`EngineCommand`])
//! and events flowing out ([`EngineEvent`]), plus the input types they carry.

use crate::cookies::Cookie;
use crate::engine::types::{Action, NavigationId, RequestId};
use crate::net::req_ref_tracker::RequestReference;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult, FetchResultMeta, Initiator, Priority, ResourceKind};
use crate::net::DecisionToken;
use crate::storage::event::StorageScope;
use crate::tab::history::{HistoryEntryId, HistorySnapshot};
use crate::tab::TabId;
use crate::zone::ZoneId;
use crate::EngineError;
use bitflags::bitflags;
use gosub_render_pipeline::render::backend::ExternalHandle;
use gosub_render_pipeline::render::Viewport;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use url::Url;

#[derive(Debug, Clone, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// The mouse cursor the page wants shown at the pointer's position. The engine reports it
/// (see [`EngineEvent::CursorChanged`]); the shell maps it to the native cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// The platform's ordinary arrow.
    #[default]
    Default,
    /// Hand: over a link (or anything else the page marks clickable).
    Pointer,
    /// I-beam: over selectable text or an editable field.
    Text,
}

/// Correlates a [`TabCommand::QueryHitTest`] with its [`EngineEvent::HitTestResult`]. Chosen
/// by the embedder; the engine echoes it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HitTestToken(pub u64);

/// Identifies one download across its progress/finished/failed events. Minted by the
/// embedder when it starts the download (like [`HitTestToken`]); the engine echoes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId(pub u64);

/// What is under a point of the page - the input for a shell's native context menu.
/// Fields are independent: a linked image yields both `link_url` and `image_url`. URLs are
/// absolute (resolved against the document URL). Selection and editable-field details grow
/// here as the engine gains focus/selection state (M1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HitTestResponse {
    /// Nearest enclosing `<a href>`.
    pub link_url: Option<String>,
    /// `<img src>` at the point (or enclosing the point).
    pub image_url: Option<String>,
    /// Content of the text node at the point, trimmed (`None` when the point is not on text).
    pub text: Option<String>,
    /// The point is inside a text-editable control (input, textarea, contenteditable).
    pub is_editable: bool,
    /// The current text selection, when the point lies within it. Always `None` until text
    /// selection lands in the engine.
    pub selection: Option<String>,
}

impl Display for MouseButton {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MouseButton::Left => write!(f, "Left"),
            MouseButton::Middle => write!(f, "Middle"),
            MouseButton::Right => write!(f, "Right"),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Modifiers: u8 {
        const SHIFT   = 0b0001;
        const CONTROL = 0b0010;
        const ALT     = 0b0100;
        const META    = 0b1000;
    }
}

impl Display for Modifiers {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();

        if self.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        if self.contains(Modifiers::CONTROL) {
            parts.push("Control");
        }
        if self.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        if self.contains(Modifiers::META) {
            parts.push("Meta");
        }

        if parts.is_empty() {
            write!(f, "None")
        } else {
            write!(f, "{}", parts.join("+"))
        }
    }
}

// Commands sent to the IO / network layer
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum IoCommand {
    Fetch {
        zone_id: ZoneId,
        req: FetchRequest,
        handle: FetchHandle,
        reply_tx: oneshot::Sender<FetchResult>,
    },
    /// Return a decision on a pending request. Tokens are process-wide unique,
    /// so no zone id is needed to route them.
    Decision { token: DecisionToken, action: Action },
    /// Ask IO to shut down a specific zone; replies when fully stopped.
    ShutdownZone {
        zone_id: ZoneId,
        reply_tx: oneshot::Sender<()>,
    },
}

/// Commands that can be sent to a specific tab
#[derive(Clone, Debug, PartialEq)]
pub enum TabCommand {
    // ****************************************
    // ** Navigation / lifecycle
    /// Navigate to specific URL
    Navigate {
        url: String,
    },
    /// Load caller-supplied HTML directly into the tab, bypassing the network.
    /// `base_url` becomes the document URL and is used to resolve relative subresources.
    LoadHtml {
        html: String,
        base_url: String,
    },
    /// Reload current URL (with or without cache)
    Reload {
        ignore_cache: bool,
    },
    /// Cancel the current navigation
    CancelNavigation,
    /// Session history: go to the previous entry (nearest navigable ancestor). No-op at the root.
    GoBack,
    /// Session history: go to a forward entry - `entry` must be one of the current entry's
    /// forward children (see [`NavigationEvent::HistoryChanged`]); `None` takes the preferred
    /// (most recently visited) one. No-op when there is no forward entry.
    GoForward {
        entry: Option<HistoryEntryId>,
    },
    /// Session history: jump to any entry in the tab's history tree.
    GoToHistoryEntry {
        entry: HistoryEntryId,
    },
    /// Save `url` to `target_path` through the zone's fetcher (cookies and UA apply).
    /// Sent to accept an [`EngineEvent::DownloadRequested`] offer, or directly
    /// (save-link-as). Progress arrives as `Download*` events carrying the same `id`.
    StartDownload {
        id: DownloadId,
        url: String,
        target_path: std::path::PathBuf,
    },
    /// Ask what is at viewport point `(x, y)` (CSS px), e.g. on right-click, to build a native
    /// context menu. Answered with [`EngineEvent::HitTestResult`] carrying the same `token`.
    QueryHitTest {
        x: f32,
        y: f32,
        token: HitTestToken,
    },
    /// Answer a pending [`NavigationEvent::DecisionRequired`].
    SubmitDecision {
        nav_id: NavigationId,
        decision_token: DecisionToken,
        action: Action,
    },
    CloseTab,

    // ****************************************
    // ** Rendering control
    /// Resume sending draw events to the tab's event channel, capped at `fps`.
    ResumeDrawing {
        fps: u16,
    },
    SuspendDrawing,
    SetViewport {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },

    // ****************************************
    // ** Tab properties
    SetTitle {
        title: String,
    },

    // ****************************************
    // ** User input
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseDown {
        x: f32,
        y: f32,
        button: MouseButton,
    },
    MouseUp {
        x: f32,
        y: f32,
        button: MouseButton,
    },
    MouseScroll {
        delta_x: f32,
        delta_y: f32,
    },
    KeyDown {
        key: String,
        code: String,
        modifiers: Modifiers,
    },
    KeyUp {
        key: String,
        code: String,
        modifiers: Modifiers,
    },
    /// Not yet handled: the tab worker logs and drops it.
    TextInput {
        text: String,
    },
    /// @TODO: needed since we have TextInput?
    CharInput {
        ch: char,
    },

    // ****************************************
    // ** Session / zone state
    /// Not yet handled: the tab worker logs and drops it.
    SetCookie {
        cookie: Cookie,
    },
    /// Not yet handled: the tab worker logs and drops it.
    ClearCookies,
    /// @TODO: local / session??
    /// Not yet handled: the tab worker logs and drops it.
    SetStorageItem {
        key: String,
        value: String,
    },
    /// Not yet handled: the tab worker logs and drops it.
    RemoveStorageItem {
        key: String,
    },
    /// Not yet handled: the tab worker logs and drops it.
    ClearStorage,

    // ****************************************
    // ** Media / scripting
    /// Execute given javascript (how about lua?)
    /// Not yet handled: the tab worker logs and drops it.
    ExecuteScript {
        source: String,
    },
    /// Not yet handled: the tab worker logs and drops it.
    PlayMedia {
        element_id: u64,
    },
    /// Not yet handled: the tab worker logs and drops it.
    PauseMedia {
        element_id: u64,
    },

    // ****************************************
    // ** Debug / devtools
    /// Not yet handled: the tab worker logs and drops it.
    DumpDomTree,
}

#[derive(Debug)]
pub enum EngineCommand {
    Shutdown {
        reply: oneshot::Sender<anyhow::Result<(), EngineError>>,
    },
}

/// Navigation events. These are the "top" events that will trigger load and resource events. All
/// events triggered in this navigation will have the same navigation id.
#[derive(Debug, Clone)]
pub enum NavigationEvent {
    Started {
        nav_id: NavigationId,
        url: Url,
    },
    /// A new document will replace the current one
    Committed {
        nav_id: NavigationId,
        url: Url,
    },
    Finished {
        nav_id: NavigationId,
        url: Url,
    },
    Failed {
        nav_id: Option<NavigationId>,
        url: Url,
        error: Arc<anyhow::Error>,
    },
    Progress {
        nav_id: NavigationId,
        received_bytes: u64,
        expected_length: Option<u64>,
        elapsed: Duration,
    },
    /// The URL string could not be parsed (as opposed to [`Self::Failed`], where the fetch failed)
    FailedUrl {
        nav_id: Option<NavigationId>,
        url: String,
        error: Arc<anyhow::Error>,
    },
    Cancelled {
        nav_id: NavigationId,
        url: Url,
        reason: CancelReason,
    },
    /// The navigation requires a decision on how to proceed (e.g., auth, certificate, block, allow);
    /// answered via [`TabCommand::SubmitDecision`]
    DecisionRequired {
        nav_id: NavigationId,
        meta: FetchResultMeta,
        decision_token: DecisionToken,
    },
    /// The tab's session history changed (entry added, back/forward moved, title learned).
    /// Carries the full snapshot so shells can update back/forward buttons and menus without
    /// querying. Emitted after the corresponding `Finished`.
    HistoryChanged {
        history: HistorySnapshot,
    },
}

/// Events triggered by load resources for a main document. Note that resources can trigger other
/// resources. @TODO: how do we see this?
///
/// Every variant carries the `request_id` of the load (@TODO: what if it contains multiple
/// redirects?) and its `reference` — what the resource belongs to (navigation id, document id,
/// background task id etc.).
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    Queued {
        request_id: RequestId,
        reference: RequestReference,
        url: String,
        kind: ResourceKind,
        initiator: Initiator,
        priority: Priority,
    },
    Started {
        request_id: RequestId,
        reference: RequestReference,
        url: String,
        kind: ResourceKind,
        initiator: Initiator,
    },
    /// Resource responded by a redirection to another resource (will trigger a new "Started")
    Redirected {
        request_id: RequestId,
        reference: RequestReference,
        from: String,
        to: String,
        /// Status code for redirection (3xx)
        status: u16,
    },
    Progress {
        request_id: RequestId,
        reference: RequestReference,
        received_bytes: u64,
        /// Expected length (based on content-length for instance)
        expected_length: Option<u64>,
        /// Time since start of the resource fetch
        elapsed: Duration,
    },
    Finished {
        request_id: RequestId,
        reference: RequestReference,
        url: Url,
        received_bytes: u64,
        /// Time spend from connection open to complete fetch of the resource
        elapsed: Option<Duration>,
    },
    Failed {
        request_id: RequestId,
        reference: RequestReference,
        url: String,
        error: Arc<anyhow::Error>,
    },
    Cancelled {
        request_id: RequestId,
        reference: RequestReference,
        url: String,
        reason: CancelReason,
    },
    Headers {
        request_id: RequestId,
        reference: RequestReference,
        url: String,
        status: u16,
        content_length: Option<u64>,
        content_type: Option<String>,
        headers: Vec<(String, String)>,
    },
}

/// Reasons for cancelling a load request
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CancelReason {
    /// user navigated away
    NewNavigation,
    TabClosed,
    /// user/UA cancelled
    ExplicitCancel,
    Timeout,
    Custom(String),
}

impl Display for CancelReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CancelReason::NewNavigation => write!(f, "Cancelling Navigation"),
            CancelReason::TabClosed => write!(f, "Cancelling tab closed"),
            CancelReason::ExplicitCancel => write!(f, "Cancelling explicit cancel"),
            CancelReason::Timeout => write!(f, "Cancelling timeout"),
            CancelReason::Custom(msg) => write!(f, "{}", msg),
        }
    }
}

/// Engine events
#[derive(Debug, Clone)]
pub enum EngineEvent {
    // ****************************************
    // ** Engine lifecycle
    EngineStarted,
    /// Not yet emitted by the engine.
    BackendChanged {
        old: String,
        new: String,
    },
    /// Not yet emitted by the engine.
    Warning {
        message: String,
    },
    /// Not yet emitted by the engine.
    EngineShutdown {
        reason: String,
    },

    // ****************************************
    // ** Zone lifecycle
    ZoneCreated {
        zone_id: ZoneId,
    },
    ZoneClosed {
        zone_id: ZoneId,
    },

    // ****************************************
    // ** Rendering
    Redraw {
        tab_id: TabId,
        handle: ExternalHandle,
    },
    /// Frame has been completed (@TODO: do we need this?)
    /// Not yet emitted by the engine.
    FrameComplete {
        tab_id: TabId,
        frame_id: u64,
    },

    // ****************************************
    // ** Tab state
    /// Hovered link URL changed (None = no link under cursor)
    HoverUrl {
        tab_id: TabId,
        url: Option<String>,
    },
    /// The cursor shape for what is under the pointer changed (emitted on change only, from
    /// mouse-move handling; resets to `Default` on navigation).
    CursorChanged {
        tab_id: TabId,
        cursor: CursorShape,
    },
    /// Keyboard focus moved to another element (or cleared). `editable` says whether the
    /// newly focused element accepts text input - the shell's cue for IME/OSK behaviour.
    FocusChanged {
        tab_id: TabId,
        /// `true` while an element is focused; `false` after a blur.
        focused: bool,
        editable: bool,
    },
    /// Answer to [`TabCommand::QueryHitTest`].
    HitTestResult {
        tab_id: TabId,
        token: HitTestToken,
        hit: HitTestResponse,
    },
    /// A navigation turned out to be a download (binary content or an attachment). The
    /// navigation itself was cancelled (the page stays); the shell decides where to save
    /// and answers with [`TabCommand::StartDownload`] - or ignores the offer.
    DownloadRequested {
        tab_id: TabId,
        url: Url,
        /// From `Content-Disposition` when present, else the URL's last path segment.
        suggested_filename: String,
        content_type: Option<String>,
        total_bytes: Option<u64>,
    },
    /// Bytes are flowing to disk for a [`TabCommand::StartDownload`].
    DownloadProgress {
        tab_id: TabId,
        id: DownloadId,
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    /// The download completed and the file is fully written.
    DownloadFinished {
        tab_id: TabId,
        id: DownloadId,
        path: std::path::PathBuf,
        received_bytes: u64,
    },
    /// The download failed; a partial file may remain at the target path.
    DownloadFailed {
        tab_id: TabId,
        id: DownloadId,
        error: String,
    },
    /// Not yet emitted by the engine; emission arrives with the pending mac-app patches.
    TitleChanged {
        tab_id: TabId,
        title: String,
    },
    /// The page's icon was fetched: raw image bytes (ICO/PNG/SVG… as served) of the first
    /// `<link rel="icon">` (or `shortcut icon` / `apple-touch-icon`) with an `href`, else
    /// `/favicon.ico`. Emitted once per committed navigation when the fetch succeeds; a
    /// navigation without a reachable icon emits nothing (the shell keeps its placeholder).
    FavIconChanged {
        tab_id: TabId,
        favicon: Vec<u8>,
    },
    /// Not yet emitted by the engine; emission arrives with the pending mac-app patches.
    LocationChanged {
        tab_id: TabId,
        url: String,
    },
    /// Not yet emitted by the engine.
    TabResized {
        tab_id: TabId,
        viewport: Viewport,
    },

    // ****************************************
    // ** Navigation

    // Navigation events (for main document)
    Navigation {
        tab_id: TabId,
        event: NavigationEvent,
    },
    /// Lowlevel resource events for all resources loaded
    Resource {
        tab_id: TabId,
        event: ResourceEvent,
    },

    // /// Redirect occurred
    // Redirect { tab_id: TabId, from: String, to: String },

    // ********************************************
    // ** Networking
    /// Not yet emitted by the engine.
    ConnectionEstablished {
        tab_id: TabId,
        url: String,
    },

    // ****************************************
    // ** Tab lifecycle
    TabCreated {
        tab_id: TabId,
        zone_id: ZoneId,
    },
    TabClosed {
        tab_id: TabId,
        zone_id: ZoneId,
    },

    // ** Tab

    // ** Session / zone state
    /// Not yet emitted by the engine.
    CookieAdded {
        tab_id: TabId,
        cookie: Cookie,
    },
    StorageChanged {
        tab_id: Option<TabId>,
        zone: Option<ZoneId>,
        key: String,
        value: Option<String>,
        scope: StorageScope,
        origin: url::Origin,
    },

    // ****************************************
    // ** Media / scripting
    /// Not yet emitted by the engine.
    MediaStarted {
        tab_id: TabId,
        element_id: u64,
    },
    /// Not yet emitted by the engine.
    MediaPaused {
        tab_id: TabId,
        element_id: u64,
    },
    /// Result of a script is returned (console stuff?)
    /// Not yet emitted by the engine.
    ScriptResult {
        tab_id: TabId,
        result: serde_json::Value,
    },

    // ****************************************
    // ** Errors / diagnostics
    /// Not yet emitted by the engine.
    NetworkError {
        tab_id: TabId,
        url: Url,
        message: String,
    },
    /// Not yet emitted by the engine.
    JavaScriptError {
        tab_id: TabId,
        message: String,
        line: u32,
        column: u32,
    },
    /// Not yet emitted by the engine.
    TabCrashed {
        tab_id: TabId,
        reason: String,
    },
    // Uncategorized / generic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mousebutton_display() {
        assert_eq!(MouseButton::Left.to_string(), "Left");
        assert_eq!(MouseButton::Middle.to_string(), "Middle");
        assert_eq!(MouseButton::Right.to_string(), "Right");
    }

    #[test]
    fn modifiers_display_empty_is_none() {
        let m = Modifiers::empty();
        assert_eq!(m.to_string(), "None");
        assert!(!m.contains(Modifiers::SHIFT));
        assert!(!m.contains(Modifiers::CONTROL));
        assert!(!m.contains(Modifiers::ALT));
        assert!(!m.contains(Modifiers::META));
    }

    #[test]
    fn modifiers_display_single() {
        assert_eq!(Modifiers::SHIFT.to_string(), "Shift");
        assert_eq!(Modifiers::CONTROL.to_string(), "Control");
        assert_eq!(Modifiers::ALT.to_string(), "Alt");
        assert_eq!(Modifiers::META.to_string(), "Meta");
    }

    #[test]
    fn modifiers_display_combo_in_order() {
        // Order should follow the push order in Display (Shift, Control, Alt, Meta)
        let all = Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META;
        assert_eq!(all.to_string(), "Shift+Control+Alt+Meta");

        let some = Modifiers::SHIFT | Modifiers::ALT;
        assert_eq!(some.to_string(), "Shift+Alt");
    }

    #[test]
    fn modifiers_bit_ops() {
        let mut m = Modifiers::empty();
        m.insert(Modifiers::SHIFT | Modifiers::CONTROL);
        assert!(m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::CONTROL));
        assert!(!m.contains(Modifiers::ALT));
        assert!(!m.contains(Modifiers::META));

        m.remove(Modifiers::SHIFT);
        assert!(!m.contains(Modifiers::SHIFT));
        assert!(m.contains(Modifiers::CONTROL));

        // No stray bits set
        let all = Modifiers::SHIFT | Modifiers::CONTROL | Modifiers::ALT | Modifiers::META;
        assert_eq!(m.bits() & !all.bits(), 0);
    }

    #[test]
    fn tabcommand_keydown_with_modifiers() {
        let mods = Modifiers::SHIFT | Modifiers::CONTROL;
        let e = TabCommand::KeyDown {
            key: "A".into(),
            code: "KeyA".into(),
            modifiers: mods,
        };

        match e {
            TabCommand::KeyDown { key, code, modifiers } => {
                assert_eq!(key, "A");
                assert_eq!(code, "KeyA");
                assert!(modifiers.contains(Modifiers::SHIFT));
                assert!(modifiers.contains(Modifiers::CONTROL));
                assert_eq!(modifiers.to_string(), "Shift+Control");
            }
            _ => panic!("Unexpected variant"),
        }
    }
}
