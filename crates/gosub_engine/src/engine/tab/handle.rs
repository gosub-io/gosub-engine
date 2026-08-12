use crate::engine::types::TabChannel;
use crate::events::TabCommand;
use crate::tab::sink::TabSink;
use crate::tab::TabId;
use crate::EngineError;
use gosub_render_pipeline::render::Viewport;
use std::sync::Arc;

/// A handle to a running [`Tab`](crate::tab): sends commands to the tab task and
/// holds the [`TabSink`] for subscribing to tab outputs. Commands to a closed tab
/// fail with [`EngineError::ChannelClosed`].
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
}
