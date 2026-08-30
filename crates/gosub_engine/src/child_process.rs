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

    let role = args.get(flag_at + 1).map(String::as_str).unwrap_or("");
    let code = run_role(role, &args[flag_at + 2..]);
    std::process::exit(code);
}

/// Run a child role - including those that need the embedder's render
/// configuration - if this process was started as one; otherwise return.
pub fn dispatch_with<C: crate::html::RenderConfiguration>() {
    DISPATCHED.store(true, std::sync::atomic::Ordering::Release);
    let args: Vec<String> = std::env::args().collect();
    let Some(flag_at) = args.iter().position(|a| a == ROLE_FLAG) else {
        return;
    };

    let role = args.get(flag_at + 1).map(String::as_str).unwrap_or("");
    let code = run_role_with::<C>(role, &args[flag_at + 2..]);
    std::process::exit(code);
}

/// Roles that need `C` (a renderer) dispatch here; every other role behaves
/// exactly as under [`dispatch`].
#[allow(clippy::extra_unused_type_parameters)]
fn run_role_with<C: crate::html::RenderConfiguration>(role: &str, args: &[String]) -> i32 {
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

    match role {
        NET_ROLE => match adopt_link(role, args) {
            Ok(endpoint) => crate::net::process::child::serve(endpoint),
            Err(code) => code,
        },
        other => {
            eprintln!("[gosub] unknown child role '{other}'");
            2
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
