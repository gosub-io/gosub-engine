//! Child-role dispatch: the one call an embedder must make for the engine to be
//! able to run components in separate processes.
//!
//! # What an embedder has to do
//!
//! ```no_run
//! fn main() {
//!     gosub_engine::child_process::dispatch();
//!     // ... normal startup from here
//! }
//! ```
//!
//! First statement in `main`, before any other work. A child process must
//! reach its role without having built windows, spawned threads or opened files
//! as a side effect of the embedder starting up. In an ordinary run the call
//! looks at `argv`, sees no role, and returns immediately.
//!
//! # Why the embedder is involved at all
//!
//! A child is created by re-exec'ing *this* binary with a role argument, so the
//! child is always the same build as the broker: nothing to locate at runtime,
//! no version skew, and no separate helper someone could replace. The cost is
//! that the engine cannot get control of the new process on its own - `main`
//! belongs to the embedder, and execution passes through it before any engine
//! code runs. This function is where the engine takes over.
//!
//! An embedder that never calls it still works; it simply cannot use process
//! isolation, and the engine says so rather than failing obscurely when a child
//! re-execs into the embedder's own startup path.
//!
//! # The argv contract
//!
//! Roles are introduced by [`ROLE_FLAG`], which is deliberately distinctive so it
//! cannot collide with an embedder's own arguments. Everything after it belongs
//! to the engine.

/// Marks the arguments that follow as an engine child role.
pub const ROLE_FLAG: &str = "--gosub-child-role";

/// Set by [`dispatch`]/[`dispatch_with`] in the broker (a child never returns from
/// them). The engine consults it before spawning anything: a child of an embedder
/// that never dispatched would re-exec into that embedder's own startup.
static DISPATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this process called [`dispatch`] or [`dispatch_with`] - the
/// precondition for process isolation. `false` in a child role by construction.
pub fn was_dispatched() -> bool {
    DISPATCHED.load(std::sync::atomic::Ordering::Acquire)
}

/// Run a child role if this process was started as one; otherwise return.
pub fn dispatch() {
    DISPATCHED.store(true, std::sync::atomic::Ordering::Release);
    let args: Vec<String> = std::env::args().collect();
    let Some(flag_at) = args.iter().position(|a| a == ROLE_FLAG) else {
        return;
    };

    let (role, rest) = split_role(&args, flag_at);
    let code = run_role(role, rest);
    std::process::exit(code);
}

/// The role after [`ROLE_FLAG`] at `flag_at` and the arguments after it. A flag
/// with nothing after it is an empty role, which [`run_role`] refuses with
/// status 2 like any unknown one.
fn split_role(args: &[String], flag_at: usize) -> (&str, &[String]) {
    let role = args.get(flag_at + 1).map(String::as_str).unwrap_or("");
    let rest = args.get(flag_at + 2..).unwrap_or(&[]);
    (role, rest)
}

/// Run a child role - including the fork server - if this process was started
/// as one; otherwise return.
pub fn dispatch_with<C: crate::html::RenderConfiguration>() {
    DISPATCHED.store(true, std::sync::atomic::Ordering::Release);
    let args: Vec<String> = std::env::args().collect();
    let Some(flag_at) = args.iter().position(|a| a == ROLE_FLAG) else {
        return;
    };

    let (role, rest) = split_role(&args, flag_at);
    let code = run_role_with::<C>(role, rest);
    std::process::exit(code);
}

#[cfg(all(feature = "process-isolation", target_os = "linux"))]
fn run_role_with<C: crate::html::RenderConfiguration>(role: &str, args: &[String]) -> i32 {
    use crate::fork_server::client::FORK_SERVER_ROLE;

    if role == FORK_SERVER_ROLE {
        gosub_sandbox::deny_debugger_attach();
        return match adopt_link(role, args) {
            Ok(endpoint) => crate::fork_server::child::serve::<C>(endpoint),
            Err(code) => code,
        };
    }
    if role == crate::render_process::client::RENDERER_ROLE {
        gosub_sandbox::deny_debugger_attach();
        return match adopt_link(role, args) {
            Ok(endpoint) => crate::render_process::child::serve::<C>(endpoint),
            Err(code) => code,
        };
    }
    run_role(role, args)
}

#[cfg(not(all(feature = "process-isolation", target_os = "linux")))]
fn run_role_with<C: crate::html::RenderConfiguration>(role: &str, args: &[String]) -> i32 {
    // No fork server here (feature off, or a platform with nothing to fork);
    // every other role behaves exactly as under `dispatch`.
    run_role(role, args)
}

/// Whether this process was started as a child role.
pub fn is_child_process() -> bool {
    std::env::args().any(|a| a == ROLE_FLAG)
}

fn run_role(role: &str, args: &[String]) -> i32 {
    use crate::net::process::client::NET_ROLE;

    // Every child is non-dumpable: a role holds cookies or page content in its
    // address space, and another process running as the same user must not be
    // able to attach and read it.
    gosub_sandbox::deny_debugger_attach();

    use crate::decoder_process::client::DECODER_ROLE;

    match role {
        DECODER_ROLE => match adopt_link(role, args) {
            Ok(endpoint) => crate::decoder_process::child::serve(endpoint),
            Err(code) => code,
        },
        NET_ROLE => match adopt_link(role, args) {
            Ok(endpoint) => crate::net::process::child::serve(endpoint, adopt_extra(role, args)),
            Err(code) => code,
        },
        #[cfg(target_os = "linux")]
        crate::cookie_vault::protocol::VAULT_ROLE => match adopt_link(role, args) {
            Ok(endpoint) => crate::cookie_vault::child::serve(endpoint, adopt_extra(role, args)),
            Err(code) => code,
        },
        #[cfg(target_os = "linux")]
        crate::storage_service::protocol::STORAGE_ROLE => match adopt_link(role, args) {
            Ok(endpoint) => match args.first().filter(|_| args.len() >= 2) {
                Some(dir) => crate::storage_service::child::serve(endpoint, std::path::PathBuf::from(dir)),
                None => {
                    eprintln!("[gosub] the storage role needs its directory argument");
                    2
                }
            },
            Err(code) => code,
        },
        other => {
            eprintln!("[gosub] unknown child role '{other}'");
            2
        }
    }
}

/// The second inherited channel, when the spawner named one before the
/// primary link. Failing to adopt it is reported and treated as absent.
fn adopt_extra(role: &str, args: &[String]) -> Option<gosub_ipc::Endpoint> {
    if args.len() < 2 {
        return None;
    }
    match gosub_ipc::Endpoint::adopt_inherited(&args[0]) {
        Ok(endpoint) => Some(endpoint),
        Err(e) => {
            eprintln!("[gosub] child role '{role}' could not adopt its second link: {e}");
            None
        }
    }
}

/// Take over the link this child inherited, or report why it could not.
fn adopt_link(role: &str, args: &[String]) -> Result<gosub_ipc::Endpoint, i32> {
    // `spawn` appends the primary link last; anything before it is a further
    // inherited channel the role knows what to do with.
    let Some(link) = args.last() else {
        eprintln!("[gosub] child role '{role}' needs an IPC link argument");
        return Err(2);
    };
    gosub_ipc::Endpoint::adopt_inherited(link).map_err(|e| {
        eprintln!("[gosub] child role '{role}' could not adopt its link: {e}");
        2
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_flag_with_no_role_is_an_empty_role_not_a_panic() {
        let args = args(&["app", ROLE_FLAG]);
        assert_eq!(split_role(&args, 1), ("", &[][..]));
    }

    #[test]
    fn the_role_and_its_arguments_are_split_off() {
        let args = args(&["app", ROLE_FLAG, "net", "a", "b"]);
        let (role, rest) = split_role(&args, 1);
        assert_eq!(role, "net");
        assert_eq!(rest, &args[3..]);
    }
}
