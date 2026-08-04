//! The fork server: the process that already has the fonts.
//!
//! Spawn cost was measured at ~3.7 ms, so cheap forking is not why this
//! process exists. It exists because font warm-up is worth doing **once**:
//! the fork server builds the configured font system, lets it prepare, and
//! every renderer forked from it inherits the warmed state copy-on-write —
//! paying nothing, whichever confinement tier applies.
//!
//! ## Consuming the confinement answer
//!
//! ## Single-threaded, deliberately

use crate::fork_server::protocol::{ConfinementTier, FromForkServer, ProofReply, ToForkServer};
use gosub_interface::font_system::{Confinement, FontSystem, TextStyle};
use gosub_ipc::Endpoint;

/// Run the fork server for the embedder's configured font system `F`.
pub fn serve<F: FontSystem + Default>(link: Endpoint) -> i32 {
    match F::confinement() {
        Confinement::Full => serve_warmed::<F>(link),
        other => decline(link, &other),
    }
}

/// The warmed zygote: build, prepare, confine per the instance answer, fork on
/// request.
fn serve_warmed<F: FontSystem + Default>(mut link: Endpoint) -> i32 {
    let mut fonts = F::default();
    // Populate lazily-built databases before asking anything of them.
    let _ = fonts.families();

    // The instance answer outranks the static one: a system whose readiness
    // depends on what preparation actually found reports it here, and the
    // sandbox tier follows the report.
    let answer = fonts.prepare_for_confinement();
    let tier = ConfinementTier::from(&answer);

    // First fork, deliberately: this process sits in a lazily-unshared PID
    // namespace, whose PID 1 is whatever forks first — and must then outlive
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
    // the answer selected" — a wrong tier mapping dies here, at startup,
    // rather than in the first forked renderer.
    gosub_sandbox::verify_fork_server_filter();
    if link.send(&FromForkServer::Ready { tier: tier.clone() }).is_err() {
        return 1;
    }

    loop {
        let request = match link.recv::<ToForkServer>() {
            Ok(msg) => msg,
            // The broker went away; nothing left to serve.
            Err(_) => return 0,
        };
        let reply = match request {
            ToForkServer::Ping => FromForkServer::Pong,
            ToForkServer::Shutdown => return 0,
            ToForkServer::ForkProof => match &tier {
                ConfinementTier::Unsupported(reason) => {
                    FromForkServer::Refused(format!("this font system cannot run isolated: {reason}"))
                }
                _ => fork_and_prove(&mut fonts, font_access),
            },
        };
        if link.send(&reply).is_err() {
            return 1;
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
            ToForkServer::ForkProof => FromForkServer::Refused(refusal.clone()),
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

/// Fork a renderer, confine it to its tier, shape with the inherited fonts,
/// and relay what it measured.
fn fork_and_prove<F: FontSystem>(fonts: &mut F, font_access: bool) -> FromForkServer {
    // A real `socketpair(2)`, created after our own lockdown — which is why
    // `socketpair` is in the fork-server filter. (`gosub_ipc::local_pair` is
    // the in-process mpsc stand-in and cannot cross a fork.)
    let (ours, theirs) = match gosub_ipc::channel::Channel::pair() {
        Ok(pair) => pair,
        Err(e) => return FromForkServer::Refused(format!("could not create the renderer link: {e}")),
    };

    match gosub_sandbox::fork_process() {
        Err(e) => FromForkServer::Refused(format!("fork failed: {e}")),
        Ok(gosub_sandbox::Forked::Child) => {
            // Split the endpoint *before* the renderer lockdown: the split is
            // an `fcntl(F_DUPFD_CLOEXEC)`, which the inherited fork-server
            // filter allows for exactly this moment and the renderer filter
            // then denies.
            drop(ours);
            let Ok(mut link) = Endpoint::from_channel(theirs) else {
                gosub_sandbox::exit_now(1);
            };
            // From here on this is a renderer: confine, then use only what was
            // inherited. The font system is the parent's, copy-on-write —
            // nothing is loaded, which is the entire point.
            gosub_sandbox::lock_down_forked_renderer(font_access);
            let (width, height) = fonts.measure(
                "Shaped in a forked renderer with inherited fonts",
                &TextStyle::new("serif", 21.0),
            );
            let ok = width > 0.0 && height > 0.0 && link.send(&ProofReply { width, height }).is_ok();
            gosub_sandbox::exit_now(if ok { 0 } else { 1 });
        }
        Ok(gosub_sandbox::Forked::Parent { pid }) => {
            // Close our copy of the child's half so a dead child reads as EOF.
            // No read timeout here: setting one is a `setsockopt` the filter
            // does not carry. A child that dies is an EOF; one that wedges is
            // bounded by the broker's reply clock, which kills this whole
            // process family rather than waiting on it.
            drop(theirs);
            let reply = match Endpoint::from_channel(ours) {
                Ok(mut link) => link.recv::<ProofReply>(),
                Err(e) => Err(e),
            };
            let status = gosub_sandbox::reap_child(pid);
            match (reply, status) {
                (Ok(proof), Ok(_)) => FromForkServer::Proof {
                    width: proof.width,
                    height: proof.height,
                },
                (Err(e), _) => FromForkServer::Refused(format!("forked renderer sent no proof: {e}")),
                (_, Err(e)) => FromForkServer::Refused(format!("forked renderer could not be reaped: {e}")),
            }
        }
    }
}
