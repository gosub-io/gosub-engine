//! Engine event types and commands.
//!
//! This module defines the core event types, commands, and enums used for communication
//! between the engine, zones, and tabs. It includes user input events, engine events,
//! and tab commands for navigation, rendering, and control.
//!
//! # Main Types
//!
//! - [`MouseButton`]: Represents mouse buttons (left, middle, right).
//! - [`Modifiers`]: Keyboard modifiers (Shift, Control, Alt, Meta).
//! - [`TabCommand`]: Commands for tab navigation and control.
//! - [`EngineCommand`]: Commands for engine control.
//! - [`EngineEvent`]: Events emitted by the engine, such as lifecycle events, rendering events, and errors.

use crate::config::LogLevel;
use crate::cookies::Cookie;
use crate::engine::types::{Action, NavigationId, RequestId};
use crate::html::DummyDocument;
use crate::net::req_ref_tracker::RequestReference;
use crate::net::types::{FetchHandle, FetchRequest, FetchResult, FetchResultMeta, Initiator, Priority, ResourceKind};
use crate::net::DecisionToken;
use crate::render::backend::ExternalHandle;
use crate::render::Viewport;
use crate::storage::event::StorageScope;
use crate::tab::TabId;
use crate::zone::ZoneId;
use crate::EngineError;
use bitflags::bitflags;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use url::Url;

/// Represents a mouse button that can be pressed or released
#[derive(Debug, Clone, PartialEq)]
pub enum MouseButton {
    /// Left mouse button pressed (or depressed)
    Left,
    /// Middle mouse button pressed (or depressed)
    Middle,
    /// Right mouse button pressed (or depressed)
    Right,
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
pub enum IoCommand {
    /// Perform a fetch of the given request
    Fetch {
        zone_id: ZoneId,
        req: FetchRequest,
        handle: FetchHandle,
        reply_tx: oneshot::Sender<FetchResult>,
    },
    /// Return a decision on a pending request
    Decision {
        zone_id: ZoneId,
        token: DecisionToken,
        action: Action,
    },
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
    Navigate { url: String },
    /// Reload current URL (with or without cache)
    Reload { ignore_cache: bool },
    /// Cancel the current navigation
    CancelNavigation,
    /// Make a decision what to do with the navigated resource
    SubmitDecision {
        nav_id: NavigationId,
        decision_token: DecisionToken,
        action: Action,
    },
    /// Close tab
    CloseTab,

    // ****************************************
    // ** Rendering control
    /// Resume sending draw events to the tab's event channel. Use fps as the refresh limit
    ResumeDrawing { fps: u16 },
    /// Suspend sending draw events
    SuspendDrawing,
    /// Set viewport
    SetViewport { x: i32, y: i32, width: u32, height: u32 },

    // ****************************************
    // ** Tab properties
    /// Set the title
    SetTitle { title: String },

    // ****************************************
    // ** User input
    /// Mouse moved to new position
    MouseMove { x: f32, y: f32 },
    /// Mouse button is pressed
    MouseDown { x: f32, y: f32, button: MouseButton },
    /// Mouse button is depressed
    MouseUp { x: f32, y: f32, button: MouseButton },
    /// Mouse scrolled up by delta
    MouseScroll { delta_x: f32, delta_y: f32 },
    /// Key has been pressed
    KeyDown {
        key: String,
        code: String,
        modifiers: Modifiers,
    },
    /// Key has been depressed
    KeyUp {
        key: String,
        code: String,
        modifiers: Modifiers,
    },
    /// Text input
    TextInput { text: String },
    /// Char input (@TODO: Needed since we have TextInput)?
    CharInput { ch: char },

    // ****************************************
    // ** Session / zone state
    /// Set a specific cookie
    SetCookie { cookie: Cookie },
    /// Clear all cookies
    ClearCookies,
    /// Set storage item (@TODO: local / session??)
    SetStorageItem { key: String, value: String },
    /// Remove storage item
    RemoveStorageItem { key: String },
    /// Clear whole storage
    ClearStorage,

    // ****************************************
    // ** Media / scripting
    /// Execute given javascript (how about lua?)
    ExecuteScript { source: String },
    /// Play media in element_id
    PlayMedia { element_id: u64 },
    /// Pause media in element_id
    PauseMedia { element_id: u64 },

    // ****************************************
    // ** Debug / devtools
    /// Enable logging
    EnableLogging { level: LogLevel },
    /// Dump dom tree
    DumpDomTree,
}

#[derive(Debug)]
pub enum TabInternalCommand {
    // Sets a document
    SetDocument { doc: Arc<DummyDocument> },
}

#[derive(Debug)]
pub enum EngineCommand {
    // ****************************************
    // ** Engine control
    /// Gracefully shutdown the engine
    Shutdown {
        reply: oneshot::Sender<anyhow::Result<(), EngineError>>,
    },

    // ****************************************
    // ** Debug / devtools
    /// Enable logging
    EnableLogging { level: LogLevel },
}

/// Navigation events. These are the "top" events that will trigger load and resource events. All
/// events triggered in this navigation will have the same navigation id.
#[derive(Debug, Clone)]
pub enum NavigationEvent {
    /// Navigation has been started
    Started { nav_id: NavigationId, url: Url },
    /// A new document will replace current one
    Committed { nav_id: NavigationId, url: Url },
    /// Finished loading the main document for this navigation
    Finished { nav_id: NavigationId, url: Url },
    /// Navigation has failed
    Failed {
        nav_id: Option<NavigationId>,
        url: Url,
        error: Arc<anyhow::Error>,
    },
    /// Progress of loading the main document for this navigation
    Progress {
        nav_id: NavigationId,
        received_bytes: u64,
        expected_length: Option<u64>,
        elapsed: Duration,
    },
    /// The URL given was invalid
    FailedUrl {
        nav_id: Option<NavigationId>,
        url: String,
        error: Arc<anyhow::Error>,
    },
    /// The navigation has been cancelled
    Cancelled {
        nav_id: NavigationId,
        url: Url,
        reason: CancelReason,
    },
    /// The navigation requires a decision on how to proceed (e.g., auth, certificate, block, allow)
    DecisionRequired {
        nav_id: NavigationId,
        meta: FetchResultMeta,
        decision_token: DecisionToken,
    },
}

/// Events triggered by load resources for a main document. Note that resources can trigger other
/// resources. @TODO: how do we see this?
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    /// Response metadata for decision on navigation
    Queued {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        /// Actual URL of the resource
        url: String,
        /// Type of resource
        kind: ResourceKind,
        /// Source that initiated this resource load
        initiator: Initiator,
        /// At which priority it is queued
        priority: Priority,
    },
    /// Loading of the resource started
    Started {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        /// Actual URL of the resource
        url: String,
        /// Type of resource
        kind: ResourceKind,
        /// Source that initiated this resource load
        initiator: Initiator,
    },
    /// Resource responded by a redirection to another resource (will trigger a new "Started")
    Redirected {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        // Redirection from this url
        from: String,
        // Redirection to this url
        to: String,
        /// Status code for redirection (3xx)
        status: u16,
    },
    /// Shows the progress of the download of the resource
    Progress {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        /// Amount of bytes received
        received_bytes: u64,
        /// Expected length (based on content-length for instance)
        expected_length: Option<u64>,
        /// Time since start of the resource fetch
        elapsed: Duration,
    },
    /// Emitted when we have finished the complete resource
    Finished {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        url: Url,
        /// Total bytes received
        received_bytes: u64,
        /// Time spend from connection open to complete fetch of the resource
        elapsed: Option<Duration>,
    },
    /// Emitted when the resource has failed loading
    Failed {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        url: String,
        /// Reason the resource fetch failed
        error: Arc<anyhow::Error>,
    },
    /// Emitted when the resource loading has been cancelled
    Cancelled {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        // Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        /// Actual URL of the resource
        url: String,
        /// Reason for cancellation
        reason: CancelReason,
    },
    Headers {
        /// Request ID of the resource load (what if it contains multiple redirects?)
        request_id: RequestId,
        /// Reference ID for this resource (navigation id, document id, background task id etc.)
        reference: RequestReference,
        /// Actual URL of the resource
        url: String,
        /// HTTP status code of the response
        status: u16,
        /// Content length if known
        content_length: Option<u64>,
        /// Content type if known
        content_type: Option<String>,
        /// All response headers
        headers: Vec<(String, String)>,
    },
}

/// Reasons for cancelling a load request
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CancelReason {
    /// user navigated away
    NewNavigation,
    /// Tab is closed
    TabClosed,
    /// user/UA cancelled
    ExplicitCancel,
    /// Timeout occurred
    Timeout,
    /// Custom reason
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
    /// Engine has started
    EngineStarted,
    /// Render backend has changed for the engine
    BackendChanged {
        old: String,
        new: String,
    },
    /// Warning from the engine
    Warning {
        message: String,
    },
    /// Engine is shutting down
    EngineShutdown {
        reason: String,
    },

    // ****************************************
    // ** Zone lifecycle
    /// Zone created
    ZoneCreated {
        zone_id: ZoneId,
    },
    /// Zone closed
    ZoneClosed {
        zone_id: ZoneId,
    },

    // ****************************************
    // ** Rendering
    /// A redraw frame is available
    Redraw {
        tab_id: TabId,
        handle: ExternalHandle,
    },
    /// Frame has been completed (@TODO: do we need this?)
    FrameComplete {
        tab_id: TabId,
        frame_id: u64,
    },

    // ****************************************
    // ** Tab state
    /// Title of the tab has changed
    TitleChanged {
        tab_id: TabId,
        title: String,
    },
    /// Favicon of tab has changed
    FavIconChanged {
        tab_id: TabId,
        favicon: Vec<u8>,
    },
    /// Location of the tab has changed
    LocationChanged {
        tab_id: TabId,
        url: String,
    },
    /// Viewport of the tab has changed
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
    /// Network connection has been established
    ConnectionEstablished {
        tab_id: TabId,
        url: String,
    },

    // ****************************************
    // ** Tab lifecycle
    /// New tab created in zone
    TabCreated {
        tab_id: TabId,
        zone_id: ZoneId,
    },
    /// Tab closed in zone
    TabClosed {
        tab_id: TabId,
        zone_id: ZoneId,
    },

    // ** Tab
    /// Title of the tab has changed
    TabTitleChanged {
        tab_id: TabId,
        title: String,
    },

    // ** Session / zone state
    /// A cookie has been added
    CookieAdded {
        tab_id: TabId,
        cookie: Cookie,
    },
    /// Storage has changed
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
    /// Media has started
    MediaStarted {
        tab_id: TabId,
        element_id: u64,
    },
    /// Media has paused
    MediaPaused {
        tab_id: TabId,
        element_id: u64,
    },
    /// Result of a script is returned (console stuff?)
    ScriptResult {
        tab_id: TabId,
        result: serde_json::Value,
    },

    // ****************************************
    // ** Errors / diagnostics
    /// Network error occurred
    NetworkError {
        tab_id: TabId,
        url: Url,
        message: String,
    },
    /// Javascript (parse) error
    JavaScriptError {
        tab_id: TabId,
        message: String,
        line: u32,
        column: u32,
    },
    /// Engine crashed
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
    fn tabcommand_equality_and_debug() {
        let a = TabCommand::SetTitle { title: "Hello".into() };
        let b = a.clone();
        assert_eq!(a, b);
        let dbg = format!("{:?}", a);
        assert!(dbg.contains("SetTitle"));
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

    #[test]
    fn tabcommand_mouse_and_resize() {
        let down = TabCommand::MouseDown {
            x: 10.0,
            y: 20.0,
            button: MouseButton::Left,
        };
        let up = TabCommand::MouseUp {
            x: 10.0,
            y: 20.0,
            button: MouseButton::Left,
        };
        let viewport = TabCommand::SetViewport {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };

        // Just basic sanity and Debug formatting
        assert!(format!("{down:?}").contains("MouseDown"));
        assert!(format!("{up:?}").contains("MouseUp"));
        assert!(format!("{viewport:?}").contains("Viewport"));
    }

    #[test]
    fn engineevent_simple_variants_debug() {
        let a = EngineEvent::EngineStarted;
        let b = EngineEvent::Warning {
            message: "Heads up".into(),
        };
        let c = EngineEvent::EngineShutdown { reason: "Bye".into() };

        assert!(format!("{a:?}").contains("EngineStarted"));
        assert!(format!("{b:?}").contains("Warning"));
        assert!(format!("{c:?}").contains("EngineShutdown"));
    }
}
