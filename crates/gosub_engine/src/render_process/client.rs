//! The broker's side of an exec'd renderer: spawn, render one page, reap.

use crate::fork_server::client::{drive_render_exchange, RenderedPage, TileMemory};
use crate::fork_server::protocol::ToForkServer;
use gosub_ipc::Endpoint;
use std::time::Duration;

/// The argv role name the broker re-execs itself with.
pub const RENDERER_ROLE: &str = "renderer";

/// How long any message in the exchange may take. Spawn plus font-system
/// setup is a few hundred ms at worst; page rendering produces a message per
/// tile, so this bounds *gaps*, not the whole render.
const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Render `html` in a fresh, throwaway, font-readable-confined renderer
/// process: spawn, send the one `RenderPage` it serves, drive the exchange
/// (subresources answered by `loader`, tiles streamed in as sealed memfds),
/// reap. The process is gone when this returns - the strongest isolation a
/// `FontPathsReadable` configuration can get, at ~4 ms of spawn cost.
pub fn render_page(
    html: &str,
    url: &str,
    tab: &str,
    viewport: (f64, f64),
    loader: &dyn gosub_interface::resource_loader::ResourceLoader,
    known_tiles: &TileMemory,
    hovered_node: Option<u64>,
) -> anyhow::Result<RenderedPage> {
    // Same guard as every spawner: an undispatched child must not recurse.
    if crate::child_process::is_child_process() {
        anyhow::bail!(
            "this process was started as an engine child role but is running embedder startup, \
             which means gosub_engine::child_process::dispatch_with() was not called at the top \
             of main(); refusing to spawn further processes"
        );
    }

    let exe = std::env::current_exe()?;
    let (ours, theirs) = gosub_ipc::channel::Channel::pair()?;

    let mut child = gosub_sandbox::spawn::spawn(
        &exe,
        &[crate::child_process::ROLE_FLAG, RENDERER_ROLE],
        theirs,
        // No network, no IPC/UTS - but the PID namespace is kept: this role's
        // font stack must be able to create threads (a PID-unshared process
        // cannot), and it never forks, so nothing is lost.
        gosub_sandbox::NamespaceIsolation::NoPidNamespace,
        gosub_sandbox::spawn::ContainerProfile {
            name: "gosub-renderer",
            internet: false,
            fs_grant: None,
        },
    )?;
    if let Err(e) = gosub_sandbox::confine_spawned_child(&child) {
        log::warn!("could not apply parent-side confinement to the renderer: {e}");
    }

    let result = (|| {
        let mut link = Endpoint::from_channel(ours)?;
        let _ = link.tx.set_write_timeout(Some(REPLY_TIMEOUT));
        let _ = link.rx.set_read_timeout(Some(REPLY_TIMEOUT));
        link.send(&ToForkServer::RenderPage {
            html: html.to_string(),
            url: url.to_string(),
            tab: tab.to_string(),
            viewport_width: viewport.0,
            viewport_height: viewport.1,
            known_tiles: known_tiles.hashes(),
            hovered_node,
        })?;
        drive_render_exchange(&mut link, loader, known_tiles)
    })();

    // Kill before reaping, on every path: a renderer that will not exit -
    // wedged or hostile - must not be able to hold this thread open.
    let _ = child.kill();
    let _ = child.wait();

    result
}
