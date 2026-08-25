use crate::engine::types::TabChannel;
use crate::events::TabCommand;
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
    /// Channel for sending commands to the tab task.
    pub cmd_tx: TabChannel,
    /// Shared sink for tab-specific outputs (e.g. rendering, events).
    pub sink: Arc<TabSink>,
}

impl std::fmt::Debug for TabHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabHandle").field("tab_id", &self.tab_id).finish()
    }
}

impl TabHandle {
    /// Send a raw [`TabCommand`] to the tab.
    ///
    /// This is the low-level method for interacting with a tab.
    /// Higher-level helpers such as [`set_title`](Self::set_title) and
    /// [`navigate`](Self::navigate) are built on top of this.
    ///
    /// # Errors
    /// Returns [`EngineError::ChannelClosed`] if the tab task is no longer running.
    pub async fn send(&self, cmd: TabCommand) -> Result<(), EngineError> {
        self.cmd_tx.send(cmd).await.map_err(|_| EngineError::ChannelClosed)?;
        Ok(())
    }

    /// Update the tab's title.
    ///
    /// This is typically reflected in the UI (e.g. the browser tab bar).
    ///
    /// # Example
    /// ```no_run,ignore
    /// tab_handle.set_title("New Title").await?;
    /// ```
    pub async fn set_title(&self, title: impl Into<String>) -> Result<(), EngineError> {
        self.send(TabCommand::SetTitle { title: title.into() }).await
    }

    /// Update the viewport of the tab.
    ///
    /// The viewport defines the visible region of the document in CSS pixels.
    /// This is usually called when the window or tab is resized.
    ///
    /// # Example
    /// ```no_run,ignore
    /// use gosub_render_pipeline::render::Viewport;
    ///
    /// let viewport = Viewport { x: 0, y: 0, width: 1280, height: 720 };
    /// tab_handle.set_viewport(viewport).await?;
    /// ```
    pub async fn set_viewport(&self, viewport: Viewport) -> Result<(), EngineError> {
        self.send(TabCommand::SetViewport {
            x: viewport.x,
            y: viewport.y,
            width: viewport.width,
            height: viewport.height,
        })
        .await
    }

    /// Navigate the tab to a new URL.
    ///
    /// This triggers a load in the tab’s context. The URL can be any supported scheme
    /// (e.g. `http://`, `https://`, `about:`, `source:`).
    ///
    /// # Example
    /// ```no_run,ignore
    /// tab_handle.navigate("https://example.com").await?;
    /// ```
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

    /// Set the scroll offset to an absolute position in CSS px. See [`TabCommand::SetScroll`].
    pub async fn set_scroll(&self, x: i32, y: i32) -> Result<(), EngineError> {
        self.send(TabCommand::SetScroll { x, y }).await
    }

    // ---- Read-side state ----
    //
    // These read a snapshot the tab worker publishes as it commits navigations, so a shell
    // building a tab strip or restoring a session does not have to replay the event stream.
    // They are synchronous and never block on the worker; the value is as of the worker's
    // last commit, which for an in-flight navigation is still the previous document.

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
}
