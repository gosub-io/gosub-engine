//! The fork server: the process that already has the fonts.
//!
//! Exists for font warm-up, not fork speed (spawn measured ~3.7 ms): the
//! warmed font system is built once and every forked renderer inherits it
//! copy-on-write, whichever confinement tier applies.

use crate::fork_server::loader::ForkedResourceLoader;
use crate::fork_server::protocol::{ConfinementTier, FromForkServer, FromRenderer, ProofReply, ToForkServer};
use crate::fork_server::{renderer, resident};
use crate::html::RenderConfiguration;
use gosub_interface::font_system::{Confinement, FontSystem, TextStyle};
use gosub_ipc::Endpoint;
use parking_lot::Mutex;
use std::os::fd::AsRawFd;
use std::sync::Arc;

/// Run the fork server for the embedder's configuration `C`.
pub fn serve<C: RenderConfiguration>(link: Endpoint) -> i32 {
    // Before any lockdown (the capture reads /proc): the anchor and every
    // forked renderer inherit the capture and can then rename themselves in
    // `ps`; this process itself reads as the fork server on both paths below.
    gosub_sandbox::capture_process_title_region();
    gosub_sandbox::set_process_title("gosub-forksrv", "gosub: renderer fork server");

    match C::FontSystem::confinement() {
        Confinement::Full => serve_warmed::<C>(link),
        other => decline(link, &other),
    }
}

/// The warmed zygote: build, prepare, confine per the instance answer, fork on
/// request.
fn serve_warmed<C: RenderConfiguration>(mut link: Endpoint) -> i32 {
    // Before the font system, which may start a thread: `TMPDIR` is set here.
    // Shared by every forked renderer, so it is granted read-only to them
    // below; only the fork server's own warm-up may stage files in it.
    let scratch = match gosub_sandbox::claim_scratch_dir("fork-server") {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("[fork-server] could not create a private scratch directory: {e}");
            return 1;
        }
    };
    let mut fonts = C::FontSystem::default();
    // Populate lazily-built databases before asking anything of them.
    let _ = fonts.families();

    // The instance answer outranks the static one: a system whose readiness
    // depends on what preparation actually found reports it here, and the
    // sandbox tier follows the report.
    let answer = fonts.prepare_for_confinement();
    let tier = ConfinementTier::from(&answer);

    // The media store is the pipeline's other piece of lazily-file-loading
    // state: SVG `<text>` goes through a system fontdb that discovers fonts
    // on first use and opens each face's file on first *use of that face*.
    // Pinned here, pre-lockdown, and inherited copy-on-write by every forked
    // renderer - the same move as the font warm-up, for the same reason.
    if !gosub_render_pipeline::common::media::SvgDecoder::pin_system_fonts() {
        eprintln!("[fork-server] system fontdb was built before it could be pinned; SVG text may fail confined");
    }
    let forked_loader = ForkedResourceLoader::disconnected();
    // Images load deferred: a render never waits on an image download; the
    // broker fetches it and re-renders when it has arrived.
    let media_store = Arc::new(gosub_render_pipeline::common::media::MediaStore::with_loader(
        forked_loader.deferred() as Arc<dyn gosub_interface::resource_loader::ResourceLoader>,
    ));
    media_store.set_synchronous_fetch(true);
    // A renderer lives under a fixed data limit and a page may carry dozens
    // of photographs: keep this much decoded, re-decode the rest on use.
    media_store.set_decoded_budget(96 * 1024 * 1024);

    // First fork, deliberately: this process sits in a lazily-unshared PID
    // namespace, whose PID 1 is whatever forks first - and must then outlive
    // every renderer, or `fork` starts failing with `ENOMEM`. Held for the
    // whole serve loop.
    let anchor = match gosub_sandbox::hold_pid_namespace_anchor() {
        Ok(anchor) => anchor,
        Err(e) => {
            eprintln!("[fork-server] could not anchor the PID namespace: {e}");
            return 1;
        }
    };
    // What a forked renderer inherits but may not keep: this process's link
    // to the broker (it could forge messages and read brokered replies) and
    // the anchor pipe (one byte kills every sibling renderer).
    let mut parent_only = link.raw_fds();
    parent_only.push(anchor.raw_fd());

    let font_access = confine_self(&answer, &scratch);

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
                _ => fork_and_prove(&mut fonts, font_access, &parent_only),
            },
            ToForkServer::SpawnRenderer { label } => match &tier {
                ConfinementTier::Unsupported(reason) => {
                    FromForkServer::Refused(format!("this font system cannot run isolated: {reason}"))
                }
                // Replies for itself (`RendererSpawned` + the link fd).
                _ => {
                    match spawn_resident::<C>(
                        &mut link,
                        &mut fonts,
                        &media_store,
                        &forked_loader,
                        font_access,
                        &parent_only,
                        &label,
                    ) {
                        Ok(()) => continue,
                        Err(e) => FromForkServer::Refused(e),
                    }
                }
            },
            // Resident renderers exit on their own schedule; only their parent
            // can collect them, and it has no other occasion to.
            ToForkServer::ReapExited => {
                let _ = gosub_sandbox::reap_exited_children();
                FromForkServer::Pong
            }
            ToForkServer::RenderPage {
                html,
                url,
                tab,
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
                        &parent_only,
                        &html,
                        &url,
                        &tab,
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

/// Fork a resident renderer (see [`resident`]) and hand the broker its end of
/// their private link: `RendererSpawned`, then the fd. The fork server keeps
/// nothing of it but a child to reap later.
fn spawn_resident<C: RenderConfiguration>(
    broker: &mut Endpoint,
    fonts: &mut Option<C::FontSystem>,
    media_store: &Arc<gosub_render_pipeline::common::media::MediaStore>,
    forked_loader: &Arc<ForkedResourceLoader>,
    font_access: bool,
    parent_only: &[i32],
    label: &str,
) -> Result<(), String> {
    let (ours, theirs) = match gosub_ipc::channel::Channel::pair() {
        Ok(pair) => pair,
        Err(e) => return Err(format!("could not create the renderer link: {e}")),
    };

    match gosub_sandbox::fork_process() {
        Err(e) => Err(format!("fork failed: {e}")),
        Ok(gosub_sandbox::Forked::Child) => {
            drop(ours);
            gosub_sandbox::close_inherited(parent_only);
            let _ = &label;
            gosub_sandbox::set_process_title(&resident::comm(), &resident::title());
            let Ok(link) = Endpoint::from_channel(theirs) else {
                gosub_sandbox::exit_now(1);
            };
            gosub_sandbox::lock_down_forked_renderer(font_access);
            let Some(owned) = fonts.take() else {
                gosub_sandbox::exit_now(1);
            };
            resident::serve::<C>(link, owned, Arc::clone(media_store), Arc::clone(forked_loader), label)
        }
        Ok(gosub_sandbox::Forked::Parent { pid }) => {
            drop(theirs);
            broker
                .send(&FromForkServer::RendererSpawned { pid })
                .map_err(|e| format!("broker unreachable: {e}"))?;
            broker
                .tx
                .send_fd(ours.raw())
                .map_err(|e| format!("could not hand the renderer link to the broker: {e}"))?;
            // `ours` drops here: the broker holds the only copy that matters.
            Ok(())
        }
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
            ToForkServer::ReapExited => FromForkServer::Pong,
            ToForkServer::ForkProof
            | ToForkServer::RenderPage { .. }
            | ToForkServer::SpawnRenderer { .. }
            | ToForkServer::Resource(_) => FromForkServer::Refused(refusal.clone()),
        };
        if link.send(&reply).is_err() {
            return 1;
        }
    }
}

/// Apply the fork-server lockdown the answer calls for; returns whether forked
/// renderers get the file-reading tier.
fn confine_self(answer: &Confinement, scratch: &std::path::Path) -> bool {
    match answer {
        Confinement::FontPathsReadable => {
            // The ruleset is inherited by every forked renderer: the scratch
            // is readable there, never a channel one site's renderer can
            // write into for another's.
            let paths = gosub_sandbox::font_filesystem_paths();
            let mut refs: Vec<(&std::path::Path, bool)> = paths.iter().map(|p| (p.as_path(), false)).collect();
            refs.push((scratch, false));
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
fn fork_confined_task<R, T>(font_access: bool, parent_only: &[i32], task: T) -> Result<R, String>
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
            gosub_sandbox::close_inherited(parent_only);
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
fn fork_and_prove<F: FontSystem>(fonts: &mut Option<F>, font_access: bool, parent_only: &[i32]) -> FromForkServer {
    let result = fork_confined_task(font_access, parent_only, || {
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
    parent_only: &[i32],
    html: &str,
    page_url: &str,
    tab: &str,
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
            gosub_sandbox::close_inherited(parent_only);
            // Fork keeps the parent's name; say who this really is.
            let _ = (&tab, &page_url);
            gosub_sandbox::set_process_title(&resident::comm(), &resident::title());
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

            let ok = resident::stream_rendered(&mut link.lock(), &tiles, &[], summary, hit_regions);
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
                        FromRenderer::NeedResource { url, deferred } => {
                            broker
                                .send(&FromForkServer::NeedResource { url, deferred })
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
                        // A one-shot renderer retains nothing and so never
                        // lets go of anything.
                        FromRenderer::Evict { .. } => {
                            return Err(std::io::Error::other("a one-shot renderer sent an eviction"));
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
