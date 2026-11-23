//! Runs V8 the way a renderer process would: forked from a locked-down fork server,
//! confined to the renderer's seccomp filter and rlimits, jitless, without threads.
//!
//! The filter is irreversible and fork needs a single-threaded caller, so this is its own
//! binary (`harness = false`) that re-executes itself as the process to confine.
//!
//! State on 2026-11-22 (GOS-468): V8 runs in a forked child, but not under the renderer's
//! confinement. It is stopped by
//!   - `uname` and `pkey_alloc` in `V8::initialize`,
//!   - `openat("/proc/self/maps")`, `prlimit64(RLIMIT_STACK)` and `sched_getaffinity` when an
//!     isolate is created and when a script runs (glibc looking up the stack bounds),
//!   - `openat("/etc/localtime")` and `readlink` the first time a script needs the time zone,
//!   - `RLIMIT_AS`: V8 reserves about 48 GiB of address space at startup, the limit is 16 GiB.
//!
//! It needs no executable memory and creates no thread. When the sandbox admits V8, flip the
//! confined expectation below.
#![allow(clippy::expect_used, clippy::print_stderr, clippy::print_stdout)]

#[cfg(target_os = "linux")]
mod probe {
    use gosub_sandbox::Forked;
    use std::process::Command;

    const CONFINED: &str = "GOSUB_V8_PROBE_CONFINED";

    /// Exercises the heap, the random source and the time zone, not just the parser.
    const SCRIPT: &str = "var a = []; for (var i = 0; i < 100000; i++) a.push({ i: i, s: 'x' + i });\
                          new Date().toString(); Math.random(); a.length / 50000";

    fn evaluate() -> bool {
        // No worker threads: a renderer may not create any, and none would survive a fork.
        let options = gosub_v8::V8Options {
            jitless: true,
            single_threaded: true,
            ..Default::default()
        };
        if !gosub_v8::V8Engine::initialize_with(&options) {
            return false;
        }

        let isolate = &mut v8::Isolate::new(Default::default());
        v8::scope!(scope, isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let Some(source) = v8::String::new(scope, SCRIPT) else {
            return false;
        };
        let result = v8::Script::compile(scope, source, None).and_then(|script| script.run(scope));

        result.and_then(|value| value.number_value(scope)) == Some(2.0)
    }

    /// The process that stands in for the fork server. Exits 0 when the renderer it forked
    /// evaluated the script.
    pub fn fork_server(confined: bool) -> ! {
        if confined {
            gosub_sandbox::apply_child_rlimits_with(1024 * 1024 * 1024).expect("rlimits");
            gosub_sandbox::lock_down_fork_server();
        }

        match gosub_sandbox::fork_process().expect("fork") {
            Forked::Child => {
                if confined {
                    gosub_sandbox::lock_down_forked_renderer(false);
                }
                gosub_sandbox::exit_now(if evaluate() { 0 } else { 1 });
            }
            Forked::Parent { pid } => {
                let status = gosub_sandbox::reap_child(pid).expect("reap");
                std::process::exit(if status == 0 { 0 } else { 1 });
            }
        }
    }

    fn run(confined: bool) -> bool {
        let exe = std::env::current_exe().expect("own path");
        Command::new(exe)
            .arg("fork-server")
            .env(CONFINED, if confined { "1" } else { "0" })
            .status()
            .expect("spawn the probe")
            .success()
    }

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().map(String::as_str) == Some("fork-server") {
            fork_server(std::env::var(CONFINED).as_deref() == Ok("1"));
        }
        // What cargo-nextest asks of a custom harness: name the tests, then run one.
        if args.iter().any(|arg| arg == "--list") {
            if !args.iter().any(|arg| arg == "--ignored") {
                println!("v8_in_a_forked_renderer: test");
            }
            return;
        }

        assert!(run(false), "jitless V8 without threads did not run in a forked child");
        eprintln!("unconfined: ok");

        assert!(
            !run(true),
            "V8 now runs under the renderer's confinement: expect success here from now on"
        );
        eprintln!("confined: blocked, as expected for now");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    probe::main();
}
