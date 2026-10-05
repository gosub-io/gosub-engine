use crate::engine::types::TabChannel;
use crate::events::{DownloadOfferId, PendingDownload, TabCommand};
use crate::tab::sink::TabSink;
use crate::tab::TabId;
use crate::EngineError;
use gosub_render_pipeline::render::Viewport;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use url::Url;

/// A handle to a running [`Tab`](crate::tab).
///
/// The `TabHandle` is returned when a new tab is created within a zone.
/// It acts as the **control interface** for the tab:
/// - Sending asynchronous commands (title updates, navigation, viewport changes).
/// - Reading the tab's current state synchronously ([`url`](Self::url),
///   [`title`](Self::title), [`can_go_back`](Self::can_go_back)).
/// - Holding a [`TabSink`], which can be used to subscribe to tab-related outputs.
///
/// Internally, commands are sent over an asynchronous [`tokio::sync::mpsc`] channel
/// to the tab task. If the tab has already been closed, commands will fail with
/// [`EngineError::ChannelClosed`].
#[derive(Clone)]
pub struct TabHandle {
    /// The unique identifier of the tab.
    pub tab_id: TabId,
    /// Channel for sending commands to the tab task. Use [`send`](Self::send) and the
    /// helpers built on it; this is plumbing, not API.
    pub(crate) cmd_tx: TabChannel,
    /// Shared sink for tab-specific outputs. Read through the accessors
    /// ([`url`](Self::url), [`title`](Self::title), ...) rather than directly.
    pub(crate) sink: Arc<TabSink>,
}

impl std::fmt::Debug for TabHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabHandle").field("tab_id", &self.tab_id).finish()
    }
}

impl TabHandle {
    /// Send a raw [`TabCommand`] to the tab. Returns [`EngineError::ChannelClosed`]
    /// if the tab task is no longer running.
    pub async fn send(&self, cmd: TabCommand) -> Result<(), EngineError> {
        self.cmd_tx.send(cmd).await.map_err(|_| EngineError::ChannelClosed)?;
        Ok(())
    }

    /// Update the tab's title.
    pub async fn set_title(&self, title: impl Into<String>) -> Result<(), EngineError> {
        self.send(TabCommand::SetTitle { title: title.into() }).await
    }

    /// Update the tab's viewport: the visible region of the document in CSS pixels.
    pub async fn set_viewport(&self, viewport: Viewport) -> Result<(), EngineError> {
        self.send(TabCommand::SetViewport {
            x: viewport.x,
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
        })
        .await
    }

    /// Navigate the tab to a new URL. Any supported scheme works
    /// (`http://`, `https://`, `about:`, `source:`).
    pub async fn navigate(&self, url: impl Into<String>) -> Result<(), EngineError> {
        self.send(TabCommand::Navigate { url: url.into() }).await
    }

    /// Session history: go to the previous entry. See [`TabCommand::GoBack`].
    pub async fn go_back(&self) -> Result<(), EngineError> {
        self.send(TabCommand::GoBack).await
    }

    /// Session history: go to the preferred forward entry. See [`TabCommand::GoForward`].
    pub async fn go_forward(&self) -> Result<(), EngineError> {
        self.send(TabCommand::GoForward { entry: None }).await
    }

    /// Load a pending download offer as the page instead. See [`TabCommand::RenderDownload`].
    pub async fn render_download(&self, offer: DownloadOfferId) -> Result<(), EngineError> {
        self.send(TabCommand::RenderDownload { offer }).await
    }

    /// Set the scroll offset to an absolute position in CSS px. See [`TabCommand::SetScroll`].
    pub async fn set_scroll(&self, x: i32, y: i32) -> Result<(), EngineError> {
        self.send(TabCommand::SetScroll { x, y }).await
    }

    // ---- Read-side state ----
    //
    // Synchronous; never blocks on the worker. The value is as of the worker's last commit,
    // so during an in-flight navigation it still describes the previous document.

    /// The tab's current document URL, or `None` before the first navigation commits.
    pub fn url(&self) -> Option<Url> {
        self.sink.url.read().clone()
    }

    /// The tab's current title. Empty until the document supplies one.
    pub fn title(&self) -> String {
        self.sink.title.read().clone()
    }

    /// Whether [`go_back`](Self::go_back) would move anywhere.
    pub fn can_go_back(&self) -> bool {
        self.sink.can_go_back.load(Ordering::Relaxed)
    }

    /// Whether [`go_forward`](Self::go_forward) would move anywhere.
    pub fn can_go_forward(&self) -> bool {
        self.sink.can_go_forward.load(Ordering::Relaxed)
    }

    /// Download offers the tab still holds the body for, oldest first. The recovery path
    /// when a [`DownloadRequested`](crate::events::EngineEvent::DownloadRequested) was lost
    /// to a lagging receiver: everything listed here can still be accepted or rendered.
    pub fn pending_downloads(&self) -> Vec<PendingDownload> {
        self.sink.pending_downloads.read().clone()
    }
}
