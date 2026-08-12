//! The fork server: the process that already has the fonts.
//!
//! Exists for font warm-up, not fork speed (spawn measured ~3.7 ms): the
//! warmed font system is built once and every forked renderer inherits it
//! copy-on-write, whichever confinement tier applies.

use crate::fork_server::loader::ForkedResourceLoader;
use crate::fork_server::protocol::{
    ConfinementTier, FromForkServer, FromRenderer, ProofReply, TileHeader, ToForkServer,
};
use crate::fork_server::renderer;
use crate::html::RenderConfiguration;
use gosub_interface::font_system::{Confinement, FontSystem, TextStyle};
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::os::fd::AsRawFd;
use std::sync::Arc;

/// Run the fork server for the embedder's configuration `C`.
pub fn serve<C: RenderConfiguration>(link: Endpoint) -> i32 {
    match C::FontSystem::confinement() {
        Confinement::Full => serve_warmed::<C>(link),
        other => decline(link, &other),
    }
}

/// The warmed zygote: build, prepare, confine per the instance answer, fork on
/// request.
fn serve_warmed<C: RenderConfiguration>(mut link: Endpoint) -> i32 {
    let mut fonts = C::FontSystem::default();
    // Populate lazily-built databases before asking anything of them.
    let _ = fonts.families();

    // The instance answer outranks the static one: a system whose readiness
    // depends on what preparation actually found reports it here, and the
    // sandbox tier follows the report.
    let answer = fonts.prepare_for_confinement();
    let tier = ConfinementTier::from(&answer);

    // The media store is the pipeline's other piece of lazily-file-loading
    // state: constructing one decodes the placeholder SVG, whose decoder
    // loads a system fontdb from disk. Built once here, pre-lockdown, and
    // inherited copy-on-write by every forked renderer - the same move as
    // the font warm-up, for the same reason.
    let forked_loader = ForkedResourceLoader::disconnected();
    let media_store = Arc::new(gosub_render_pipeline::common::media::MediaStore::with_loader(
        Arc::clone(&forked_loader) as Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    ));
    media_store.set_synchronous_fetch(true);

    // First fork, deliberately: this process sits in a lazily-unshared PID
    // namespace, whose PID 1 is whatever forks first - and must then outlive
    // every renderer, or `fork` starts failing with `ENOMEM`. Held for the
    // whole serve loop.
    let _anchor = match gosub_sandbox::hold_pid_namespace_anchor() {
        Ok(anchor) => anchor,
        Err(e) => {
            eprintln!("[fork-server] could not anchor the PID namespace: {e}");
            return 1;
        }
    };

    let font_access = confine_self(&answer);

    // Announced only now, so `Ready` also means "still alive under the filter
    // the answer selected" - a wrong tier mapping dies here, at startup,
    // rather than in the first forked renderer.
    gosub_sandbox::verify_fork_server_filter();
    if link.send(&FromForkServer::Ready { tier: tier.clone() }).is_err() {
        return 1;
    }

    // In an `Option` so a forked child can take ownership of its
    // copy-on-write copy (the pipeline wants the font system behind an
    // `Arc<Mutex<dyn FontSystem>>`). The parent's slot is never taken.
    let mut fonts = Some(fonts);

    loop {
        let request = match link.recv::<ToForkServer>() {
            Ok(msg) => msg,
            // The broker went away; nothing left to serve.
            Err(_) => return 0,
        };
        let reply = match request {
            ToForkServer::Ping => FromForkServer::Pong,
            ToForkServer::Shutdown => return 0,
            // Resource replies only make sense inside a RenderPage exchange,
            // where the relay loop consumes them; one arriving here is a
            // confused broker.
            ToForkServer::Resource(_) => FromForkServer::Refused("resource reply with no render in flight".into()),
            ToForkServer::ForkProof => match &tier {
                ConfinementTier::Unsupported(reason) => {
                    FromForkServer::Refused(format!("this font system cannot run isolated: {reason}"))
                }
                _ => fork_and_prove(&mut fonts, font_access),
            },
            ToForkServer::RenderPage {
                html,
                url,
                viewport_width,
                viewport_height,
                known_tiles,
                hovered_node,
            } => match &tier {
                ConfinementTier::Unsupported(reason) => {
                    FromForkServer::Refused(format!("this font system cannot run isolated: {reason}"))
                }
                // Streamed reply (tiles relayed as they arrive, then the
                // summary), so this arm sends for itself - a `Refused` after
                // partial tiles tells the broker to discard them.
                _ => {
                    let outcome = fork_and_render::<C>(
                        &mut link,
                        &mut fonts,
                        &media_store,
                        &forked_loader,
                        font_access,
                        &html,
                        &url,
                        viewport_width,
                        viewport_height,
                        &known_tiles.iter().copied().collect(),
                        hovered_node,
                    );
                    match outcome {
                        Ok(()) => continue, // PageRendered already sent
                        Err(e) => FromForkServer::Refused(e),
                    }
                }
            },
        };
        if link.send(&reply).is_err() {
            return 1;
        }
    }
}

/// The header for a tile the broker already holds. Its physical dimensions
/// are left at zero deliberately: nothing rasterized them here, and the
/// broker fills them from the pixels it kept.
fn unchanged_header(page_x: f64, page_y: f64, layer_id: u64, hash: u64) -> TileHeader {
    TileHeader {
        page_x,
        page_y,
        layer_id,
        width: 0,
        height: 0,
        format: crate::fork_server::protocol::TileWireFormat::PreMulArgb32,
        content_hash: hash,
        opacity: 1.0,
        anchor: crate::fork_server::protocol::TileWireAnchor::Scroll,
    }
}

/// A tier with no use for a zygote: report the answer so the broker can act on
/// it, refuse to fork, and idle confined.
fn decline(mut link: Endpoint, answer: &Confinement) -> i32 {
    // Idle or not, this process gets no more privilege than it needs; the
    // plain fork-server filter is a superset of "answer pings".
    gosub_sandbox::lock_down_fork_server();
    if link
        .send(&FromForkServer::Ready {
            tier: ConfinementTier::from(answer),
        })
        .is_err()
    {
        return 1;
    }

    let refusal = match answer {
        Confinement::FontPathsReadable => "this tier has no use for a fork server: renderers are spawned \
             fresh and confine themselves with the font-readable profile"
            .to_string(),
        Confinement::Unsupported(reason) => format!("this font system cannot run isolated: {reason}"),
        Confinement::Full => unreachable!("Full is served by the warmed zygote"),
    };

    loop {
        let request = match link.recv::<ToForkServer>() {
            Ok(msg) => msg,
            Err(_) => return 0,
        };
        let reply = match request {
            ToForkServer::Ping => FromForkServer::Pong,
            ToForkServer::Shutdown => return 0,
            ToForkServer::ForkProof | ToForkServer::RenderPage { .. } | ToForkServer::Resource(_) => {
                FromForkServer::Refused(refusal.clone())
            }
        };
        if link.send(&reply).is_err() {
            return 1;
        }
    }
}

/// Apply the fork-server lockdown the answer calls for; returns whether forked
/// renderers get the file-reading tier.
fn confine_self(answer: &Confinement) -> bool {
    match answer {
        Confinement::FontPathsReadable => {
            // The scratch must exist and be named before Landlock freezes the
            // ruleset; `TMPDIR` points file-staging backends into it.
            let scratch = std::env::temp_dir().join(format!("gosub-fork-server-scratch-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&scratch);
            std::env::set_var("TMPDIR", &scratch);

            let paths = gosub_sandbox::font_filesystem_paths();
            let mut refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
            refs.push((scratch.as_path(), true));
            gosub_sandbox::lock_down_fork_server_with_font_access(&refs);
            true
        }
        // `Unsupported` still confines: an idle process holds page-shaping
        // machinery and deserves no more privilege for being useless.
        Confinement::Full | Confinement::Unsupported(_) => {
            gosub_sandbox::lock_down_fork_server();
            false
        }
    }
}

/// Fork a renderer, confine it to its tier, run `task` in it, and return what
/// the child sent back over their private pair.
fn fork_confined_task<R, T>(font_access: bool, task: T) -> Result<R, String>
where
    R: serde::Serialize + serde::de::DeserializeOwned,
    T: FnOnce() -> Option<R>,
{
    let (ours, theirs) = match gosub_ipc::channel::Channel::pair() {
        Ok(pair) => pair,
        Err(e) => return Err(format!("could not create the renderer link: {e}")),
    };

    match gosub_sandbox::fork_process() {
        Err(e) => Err(format!("fork failed: {e}")),
        Ok(gosub_sandbox::Forked::Child) => {
            drop(ours);
            let Ok(mut link) = Endpoint::from_channel(theirs) else {
                gosub_sandbox::exit_now(1);
            };
            // From here on this is a renderer: confine, then use only what
            // was inherited copy-on-write.
            gosub_sandbox::lock_down_forked_renderer(font_access);
            let code = match task() {
                Some(reply) if link.send(&reply).is_ok() => 0,
                _ => 1,
            };
            gosub_sandbox::exit_now(code);
        }
        Ok(gosub_sandbox::Forked::Parent { pid }) => {
            // Close our copy of the child's half so a dead child reads as EOF.
            drop(theirs);
            let reply = match Endpoint::from_channel(ours) {
                Ok(mut link) => link.recv::<R>(),
                Err(e) => Err(e),
            };
            let status = gosub_sandbox::reap_child(pid);
            match (reply, status) {
                (Ok(reply), Ok(_)) => Ok(reply),
                (Err(e), _) => Err(format!("forked renderer sent no reply: {e}")),
                (_, Err(e)) => Err(format!("forked renderer could not be reaped: {e}")),
            }
        }
    }
}

/// The smallest fork: shape one line with the inherited fonts and report the
/// measured box.
fn fork_and_prove<F: FontSystem>(fonts: &mut Option<F>, font_access: bool) -> FromForkServer {
    let result = fork_confined_task(font_access, || {
        let fonts = fonts.as_mut()?;
        let (width, height) = fonts.measure(
            "Shaped in a forked renderer with inherited fonts",
            &TextStyle::new("serif", 21.0),
        );
        (width > 0.0 && height > 0.0).then_some(ProofReply { width, height })
    });
    match result {
        Ok(proof) => FromForkServer::Proof {
            width: proof.width,
            height: proof.height,
        },
        Err(e) => FromForkServer::Refused(e),
    }
}

/// The renderer role: run the pipeline over a page in a forked, confined
/// child, seal each rasterized tile into a memfd there, and collect the lot -
/// relaying the renderer's subresource requests to the broker along the way.
#[allow(clippy::too_many_arguments)] // the serve loop's context, spelled out
fn fork_and_render<C: RenderConfiguration>(
    broker: &mut Endpoint,
    fonts: &mut Option<C::FontSystem>,
    media_store: &Arc<gosub_render_pipeline::common::media::MediaStore>,
    forked_loader: &Arc<ForkedResourceLoader>,
    font_access: bool,
    html: &str,
    page_url: &str,
    viewport_width: f64,
    viewport_height: f64,
    known_tiles: &std::collections::HashSet<u64>,
    hovered_node: Option<u64>,
) -> Result<(), String> {
    let (ours, theirs) = match gosub_ipc::channel::Channel::pair() {
        Ok(pair) => pair,
        Err(e) => return Err(format!("could not create the renderer link: {e}")),
    };

    match gosub_sandbox::fork_process() {
        Err(e) => Err(format!("fork failed: {e}")),
        Ok(gosub_sandbox::Forked::Child) => {
            drop(ours);
            let Ok(link) = Endpoint::from_channel(theirs) else {
                gosub_sandbox::exit_now(1);
            };
            gosub_sandbox::lock_down_forked_renderer(font_access);

            // The link is shared between the loader (mid-render round trips)
            // and the final result send - safe because this process is
            // single-threaded and strictly alternates.
            let link = Arc::new(Mutex::new(link));
            forked_loader.connect(Arc::clone(&link));

            let Some(owned) = fonts.take() else {
                gosub_sandbox::exit_now(1);
            };
            let shared: Arc<Mutex<dyn FontSystem>> = Arc::new(Mutex::new(owned));
            let (summary, tiles, hit_regions) = renderer::render_page::<C>(
                renderer::PageRequest {
                    html,
                    page_url,
                    viewport_width,
                    viewport_height,
                    known_tiles,
                    hovered_node,
                },
                shared,
                Arc::clone(media_store),
                Arc::clone(forked_loader) as Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
            );

            // Stream every CPU tile out: seal into an immutable memfd, send
            // header + fd, drop - one at a time, so this process never holds
            // more than one tile fd regardless of page height. All of it -
            // memfd_create, ftruncate, mmap, the seals - is in the renderer
            // baseline precisely so a confined renderer can hand off pixels.
            let mut link = link.lock();
            for tile in &tiles {
                let sent = match tile {
                    renderer::RenderedTile::Unchanged {
                        page_x,
                        page_y,
                        layer_id,
                        hash,
                    } => link
                        .send(&FromRenderer::TileUnchanged(unchanged_header(
                            *page_x, *page_y, *layer_id, *hash,
                        )))
                        .is_ok(),
                    renderer::RenderedTile::Fresh { tile, hash } => {
                        let gosub_render_pipeline::common::texture::TilePixels::Cpu(bytes) = &tile.pixels else {
                            // GPU handles are process-local and cannot cross;
                            // a forked rasterizer should never produce one.
                            continue;
                        };
                        let Ok(fd) = gosub_ipc::shm::create_sealed_tile(tile.width, tile.height, |buf| {
                            let n = buf.len().min(bytes.len());
                            buf[..n].copy_from_slice(&bytes[..n]);
                        }) else {
                            gosub_sandbox::exit_now(1);
                        };
                        let header = TileHeader {
                            page_x: tile.page_x,
                            page_y: tile.page_y,
                            layer_id: tile.layer_id,
                            width: tile.width,
                            height: tile.height,
                            format: tile.format.into(),
                            content_hash: *hash,
                            opacity: tile.opacity,
                            anchor: tile.anchor.into(),
                        };
                        link.send(&FromRenderer::Tile(header)).is_ok() && link.tx.send_fd(fd.as_raw_fd()).is_ok()
                        // `fd` drops here: SCM_RIGHTS duplicated it onward.
                    }
                };
                if !sent {
                    gosub_sandbox::exit_now(1);
                }
            }

            let ok = link.send(&FromRenderer::Rendered { summary, hit_regions }).is_ok();
            gosub_sandbox::exit_now(if ok { 0 } else { 1 });
        }
        Ok(gosub_sandbox::Forked::Parent { pid }) => {
            drop(theirs);
            // Same EOF-or-broker-clock error model as `fork_confined_task`,
            // now a pure relay: resource requests go broker-ward and back,
            // tiles go broker-ward one fd at a time (received, forwarded,
            // dropped), and the final summary closes the exchange. Strict
            // alternation on both links; O(1) fds held here.
            let relayed = (|| -> std::io::Result<()> {
                let mut link = Endpoint::from_channel(ours)?;
                loop {
                    match link.recv::<FromRenderer>()? {
                        FromRenderer::NeedResource { url } => {
                            broker
                                .send(&FromForkServer::NeedResource { url })
                                .map_err(|e| std::io::Error::other(format!("broker unreachable: {e}")))?;
                            match broker
                                .recv::<ToForkServer>()
                                .map_err(|e| std::io::Error::other(format!("broker sent no resource: {e}")))?
                            {
                                ToForkServer::Resource(reply) => link.send(&reply)?,
                                other => {
                                    return Err(std::io::Error::other(format!(
                                        "expected a resource from the broker, got {other:?}"
                                    )))
                                }
                            }
                        }
                        FromRenderer::TileUnchanged(header) => {
                            broker
                                .send(&FromForkServer::TileUnchanged(header))
                                .map_err(|e| std::io::Error::other(format!("broker unreachable: {e}")))?;
                        }
                        FromRenderer::Tile(header) => {
                            let fd = link.rx.recv_fd()?;
                            broker
                                .send(&FromForkServer::Tile(header))
                                .map_err(|e| std::io::Error::other(format!("broker unreachable: {e}")))?;
                            broker
                                .tx
                                .send_fd(fd.as_raw_fd())
                                .map_err(|e| std::io::Error::other(format!("could not relay a tile fd: {e}")))?;
                            // `fd` drops here; the broker holds the only copy.
                        }
                        FromRenderer::Rendered { summary, hit_regions } => {
                            broker
                                .send(&FromForkServer::PageRendered { summary, hit_regions })
                                .map_err(|e| std::io::Error::other(format!("broker unreachable: {e}")))?;
                            return Ok(());
                        }
                    }
                }
            })();
            let status = gosub_sandbox::reap_child(pid);
            match (relayed, status) {
                (Ok(()), Ok(_)) => Ok(()),
                (Err(e), _) => Err(format!("forked renderer died mid-render: {e}")),
                (_, Err(e)) => Err(format!("forked renderer could not be reaped: {e}")),
            }
        }
    }
}
