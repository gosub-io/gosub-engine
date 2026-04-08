use crate::engine::types::TabChannel;
use crate::events::TabCommand;
use crate::render::Viewport;
use crate::tab::sink::TabSink;
use crate::tab::TabId;
use crate::EngineError;
use std::sync::Arc;

/// A handle to a running [`Tab`](crate::tab).
///
/// The `TabHandle` is returned when a new tab is created within a zone.
/// It acts as the **control interface** for the tab:
/// - Sending asynchronous commands (title updates, navigation, viewport changes).
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
        f.debug_struct("TabHandle")
            .field("tab_id", &self.tab_id)
            .finish()
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
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_| EngineError::ChannelClosed)?;
        Ok(())
    }

    /// Update the tab's title.
    ///
    /// This is typically reflected in the UI (e.g. the browser tab bar).
    ///
    /// # Example
    /// ```no_run,ignore
    /// tab_handle.set_title("New Title");
    /// ```
    pub async fn set_title(&self, title: impl Into<String>) -> Result<(), EngineError> {
        self.send(TabCommand::SetTitle { title: title.into() })
            .await
    }

    /// Update the viewport of the tab.
    ///
    /// The viewport defines the visible region of the document in CSS pixels.
    /// This is usually called when the window or tab is resized.
    ///
    /// # Example
    /// ```no_run,ignore
    /// use gosub_engine::render::Viewport;
    ///
    /// let viewport = Viewport { x: 0.0, y: 0.0, width: 1280.0, height: 720.0 };
    /// tab_handle.set_viewport(viewport);
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
    /// tab_handle.navigate_to("https://example.com");
    /// ```
    pub async fn navigate(&self, url: impl Into<String>) -> Result<(), EngineError> {
        self.send(TabCommand::Navigate { url: url.into() }).await
    }
}
