//! Sandbox enforcement probes: the suite that proves the confinement actually
//! *binds*, rather than merely being installed.
//!
//! Each test execs the `sandbox-probe` binary, which applies a lockdown to
//! itself and then attempts one operation. The verdict is read from outside:
//! on Linux a forbidden syscall is a fatal `SIGSYS`, while platforms whose
//! denials are not fatal (macOS Seatbelt returns `EPERM`) have the probe report
//! an exit code instead. This cannot live in a `#[cfg(test)]` unit test — the
//! filters are irreversible and would kill the test runner.

// Test harness: a probe that cannot even be spawned must fail the test loudly,
// so panicking is the intended outcome rather than a defect to route around.
#![allow(clippy::expect_used)]

/// Path to the compiled probe binary, provided by Cargo to integration tests.
#[cfg(feature = "multi-process")]
fn probe_bin() -> &'static str {
    env!("CARGO_BIN_EXE_sandbox-probe")
}

/// Seatbelt must *enforce*, not just install.
///
/// Unlike seccomp, a Seatbelt denial is not fatal — the call returns `EPERM`
/// and the process continues — so these probes cannot be judged from a signal.
/// Each performs its operation before and after `sandbox_init` and reports the
/// transition through an exit code; the codes below are what those mean.
#[cfg(all(feature = "multi-process", target_os = "macos"))]
mod seatbelt_enforcement {
    use super::probe_bin;
    use std::process::Command;

    /// Mirrors `selftest::code`.
    const CONTROL_FAILED: i32 = 90;
    const NOT_DENIED: i32 = 91;
    const WRONG_ERROR: i32 = 92;
    const WRONG_VALUE: i32 = 93;

    fn probe(name: &str) -> i32 {
        let st = Command::new(probe_bin())
            .arg(name)
            .status()
            .expect("spawn sandbox-probe");
        st.code()
            .unwrap_or_else(|| panic!("{name}: killed by a signal, expected an exit code"))
    }

    /// Turn a probe's exit code into a message that says which half failed —
    /// "the sandbox let it through" and "it never worked anyway" are very
    /// different bugs and a bare assertion cannot tell them apart.
    fn check(name: &str) {
        match probe(name) {
            0 => {}
            CONTROL_FAILED => panic!(
                "{name}: the operation already failed BEFORE lockdown, so this proves \
                 nothing about the profile — the control is broken, not the sandbox"
            ),
            NOT_DENIED => panic!("{name}: the operation SUCCEEDED under the profile — not enforcing"),
            WRONG_ERROR => panic!("{name}: denied, but not with EPERM — something else refused it"),
            WRONG_VALUE => panic!("{name}: the cap was applied but did not take the expected value"),
            other => panic!("{name}: unexpected exit {other}"),
        }
    }

    /// A renderer has no filesystem — `(deny default)` withholding `file-read*`
    /// is the SBPL counterpart of `openat` being off the seccomp list.
    #[test]
    fn renderer_cannot_open_files() {
        check("seatbelt-file");
    }

    /// A renderer has no network. On Linux that is an empty netns plus missing
    /// socket syscalls; here it is the profile omitting `network-outbound`, so
    /// it needs its own test rather than inheriting the Linux one's assurance.
    #[test]
    fn renderer_cannot_reach_the_network() {
        check("seatbelt-network");
    }

    /// No new programs, the analogue of `execve`/`clone` being off the list.
    #[test]
    fn renderer_cannot_spawn_programs() {
        check("seatbelt-exec");
    }

    /// The control for the test above it: the net component's profile *does*
    /// grant outbound access. Without this, "the renderer cannot reach the
    /// network" would be equally satisfied by a machine with no network.
    #[test]
    fn net_component_keeps_its_network() {
        check("seatbelt-net-role-keeps-network");
    }

    /// The control for every denial in this module: ordinary work must still
    /// run under the profile. A profile so tight the renderer cannot function
    /// would satisfy every negative test above and ship a broken component.
    #[test]
    fn ordinary_work_survives_the_profile() {
        check("seatbelt-baseline");
    }

    /// `file-read*` and `file-write*` are separate SBPL operations — denying
    /// reads does not imply denying writes.
    #[test]
    fn renderer_cannot_write_files() {
        check("seatbelt-file-write");
    }

    /// `process-fork` is distinct from `process-exec`: forking without exec is
    /// still process creation.
    #[test]
    fn renderer_cannot_fork() {
        check("seatbelt-fork");
    }

    /// Tests the profile's *precision*, not merely that one exists. The grant
    /// is `(allow signal (target self))`; if that scope were widened or lost,
    /// every other test here would still pass and only this one would notice.
    #[test]
    fn renderer_cannot_signal_other_processes() {
        check("seatbelt-signal-other");
    }

    /// The backend docs claim the profile grants no `sysctl-read`. Nothing
    /// checked that until now; sysctls leak host details useful for
    /// fingerprinting and exploit tuning.
    #[test]
    fn renderer_cannot_read_sysctls() {
        check("seatbelt-sysctl");
    }

    /// The backend docs claim the profile grants no `mach-lookup` — reach into
    /// the Mach bootstrap namespace (WindowServer, launchd services), the classic
    /// macOS sandbox-escape surface. The probe confirms a service that resolves
    /// before lockdown is refused after; if none of its candidate services resolve
    /// in the bootstrap namespace it reports `CONTROL_FAILED` (the control is
    /// broken, not the sandbox), exactly like the other seatbelt probes.
    #[test]
    fn renderer_cannot_look_up_mach_services() {
        check("seatbelt-mach-lookup");
    }

    /// A filesystem service is path-scoped to its own directory — read/write
    /// inside, denied outside — the SBPL counterpart of the Linux services'
    /// Landlock ruleset, so a compromised storage/font service cannot roam the
    /// disk despite being a filesystem-service profile.
    #[test]
    fn service_filesystem_is_path_scoped() {
        check("seatbelt-service-scope");
    }

    /// The rlimits are a mechanism wholly separate from Seatbelt, and were
    /// entirely unverified on macOS.
    #[test]
    fn child_rlimits_are_applied() {
        check("rlimits");
    }

    /// Verifies the kernel *accepts* `PT_DENY_ATTACH` — deliberately weaker
    /// than the Linux `children_refuse_debugger_attach`, which proves an attach
    /// is actually refused.
    #[test]
    fn ptrace_deny_attach_is_accepted() {
        check("ptrace-deny-accepted");
    }
}

/// Windows process mitigation policies must *enforce*, not merely install.
#[cfg(all(feature = "multi-process", target_os = "windows"))]
mod mitigation_enforcement {
    use super::probe_bin;
    use std::process::Command;

    /// Mirrors `selftest::wcode`.
    const CONTROL_FAILED: i32 = 90;
    const NOT_DENIED: i32 = 91;
    const WRONG_VALUE: i32 = 93;

    fn check(name: &str) {
        let st = Command::new(probe_bin())
            .arg(name)
            .status()
            .expect("spawn sandbox-probe");
        match st.code().unwrap_or_else(|| panic!("{name}: no exit code")) {
            0 => {}
            CONTROL_FAILED => panic!(
                "{name}: the operation already failed BEFORE lockdown, so this proves \
                 nothing about the policy — the control is broken, not the sandbox"
            ),
            NOT_DENIED => panic!("{name}: the operation SUCCEEDED under the policy — not enforcing"),
            WRONG_VALUE => panic!("{name}: the kernel did not record the policy we set"),
            other => panic!("{name}: unexpected exit {other}"),
        }
    }

    /// The control for the denials below: ordinary work must still run. A
    /// policy set that broke the component would satisfy every negative test
    /// while shipping a renderer that cannot render.
    #[test]
    fn ordinary_work_survives_the_policies() {
        check("mitigation-baseline");
    }

    /// W^X: the counterpart of the seccomp `PROT_EXEC` argument filter, and the
    /// step most memory-corruption chains need to execute injected code.
    #[test]
    fn renderer_cannot_allocate_executable_memory() {
        check("mitigation-dynamic-code");
    }

    /// The analogue of `execve`/`clone` being absent from the allowlist.
    #[test]
    fn renderer_cannot_spawn_child_processes() {
        check("mitigation-child-process");
    }

    /// Behaviour is the real test, but the kernel's own readback catches a
    /// policy word assembled wrongly — including extension-point disabling,
    /// which has no convenient behavioural probe (it would need a third party
    /// to attempt an injection).
    #[test]
    fn kernel_recorded_the_policies() {
        check("mitigation-policies-readback");
    }

    /// Integrity is mandatory access control: a low-integrity process cannot
    /// write to objects labelled medium or above, which is most of the user's
    /// profile. This is the largest single reduction in blast radius available
    /// on Windows without a bespoke spawn path.
    #[test]
    fn renderer_cannot_write_to_medium_integrity_objects() {
        check("low-integrity");
    }

    /// The job object's memory ceiling — the `RLIMIT_AS` analogue Windows
    /// otherwise lacks, and the one parent-side control that can be attached
    /// to a process that already exists.
    #[test]
    fn job_object_caps_memory() {
        check("job-memory-limit");
    }

    /// The token handed to a child must carry fewer privileges than the one
    /// the engine runs with. Privileges are the ambient ACL-override rights; a
    /// renderer needs none of them.
    #[test]
    fn child_token_drops_privileges() {
        check("restricted-token");
    }

    /// A *spawned* child must run under the restricted token, not fall back to
    /// the inherited one.
    #[test]
    fn spawned_children_get_a_restricted_token() {
        let out = super::run(&[]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("using inherited token"),
            "a child fell back to the inherited token — restricted_token() failed:\n{stderr}"
        );
    }
}

/// Guards the enforcement suite against silently shrinking.
#[cfg(feature = "multi-process")]
mod probe_inventory {
    use super::probe_bin;
    use std::process::Command;

    /// What this platform is expected to verify. Keep in sync with
    /// `selftest::PROBES` — that is the point of the test.
    #[cfg(target_os = "linux")]
    const EXPECTED: &[&str] = &[
        "baseline",
        "mprotect-exec",
        "socket",
        "memfd-seal",
        "fcntl-dupfd",
        "ring",
        "netns",
        "pidns",
        "no-ptrace",
        "forkserver-can-fork",
        "forkserver-canary-gap",
        "forkserver-no-exec",
        "forkserver-no-socket",
        "forkserver-no-newuser-clone",
        "service-fs-openat",
        "service-fs-no-socket",
        "service-device-ioctl",
        "service-landlock",
        "broker-landlock",
        "broker-seccomp",
        "broker-seccomp-mount",
        "cgroup-memory-limit",
        "crash-report",
    ];

    /// The Seatbelt profile's enforcement. `PT_DENY_ATTACH` and the rlimits
    /// are still unprobed — the list says so rather than implying coverage.
    #[cfg(target_os = "macos")]
    const EXPECTED: &[&str] = &[
        "seatbelt-file",
        "seatbelt-network",
        "seatbelt-exec",
        "seatbelt-net-role-keeps-network",
        "seatbelt-baseline",
        "seatbelt-file-write",
        "seatbelt-fork",
        "seatbelt-signal-other",
        "seatbelt-sysctl",
        "seatbelt-mach-lookup",
        "seatbelt-service-scope",
        "rlimits",
        "ptrace-deny-accepted",
    ];

    /// Windows: the process mitigation policies. The access-confining half
    /// (restricted token, AppContainer, job object) is parent-side and not
    /// implemented, so there is nothing yet to probe for file or network
    /// confinement — see `sandbox/windows.rs`.
    #[cfg(target_os = "windows")]
    const EXPECTED: &[&str] = &[
        "mitigation-baseline",
        "mitigation-dynamic-code",
        "mitigation-child-process",
        "mitigation-policies-readback",
        "low-integrity",
        "job-memory-limit",
        "restricted-token",
    ];

    /// Everything else has no sandbox backend: components run unconfined under
    /// `sandbox::unsupported`. Nothing to probe until a measure lands — and
    /// when one does, this list is what forces a probe to land with it.
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    const EXPECTED: &[&str] = &[];

    #[test]
    fn compiled_probes_match_this_platform() {
        let out = Command::new(probe_bin())
            .arg("list")
            .output()
            .expect("spawn sandbox-probe");
        assert!(out.status.success(), "sandbox-probe list failed: {out:?}");
        let got: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        assert_eq!(
            got, EXPECTED,
            "sandbox probe inventory changed.\n\
             If you added a measure, add a probe AND a test for it, then update EXPECTED.\n\
             If a probe vanished, a `cfg` is hiding it — that is the bug this test exists to catch."
        );
    }
}

/// The sandbox must *enforce*, not just announce. These run the `selftest`
/// probes in a child that applies the renderer lockdown and then attempts one
/// operation; a forbidden op is killed by `SIGSYS`, an allowed one exits clean.
#[cfg(all(feature = "multi-process", target_os = "linux"))]
mod sandbox_enforcement {
    use super::probe_bin;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    /// `SIGSYS` — the signal seccomp `KillProcess` terminates with.
    const SIGSYS: i32 = 31;
    /// `SIGSEGV` — a segmentation fault (the crash-report probe's wild read).
    const SIGSEGV: i32 = 11;

    fn probe(name: &str) -> std::process::ExitStatus {
        Command::new(probe_bin())
            .arg(name)
            .status()
            .expect("spawn sandbox-probe")
    }

    #[test]
    fn baseline_program_survives_the_sandbox() {
        // Sanity: normal work isn't killed, so a kill below means the op, not
        // the lockdown itself, was the cause.
        let st = probe("baseline");
        assert!(st.success(), "baseline should exit cleanly, got {st:?}");
    }

    #[test]
    fn making_memory_executable_is_killed() {
        let st = probe("mprotect-exec");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (W^X), got {st:?}");
        assert!(st.code().is_none(), "should be killed, not exit");
    }

    #[test]
    fn opening_a_socket_is_killed() {
        let st = probe("socket");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (no network), got {st:?}");
    }

    /// The inbound direction: other software running as the same user must not
    /// be able to `ptrace`-attach or read `/proc/<pid>/mem`. Guards the
    /// placement as much as the call — the dumpable flag does not survive
    /// `execve`, so setting it pre-exec would leave this silently at 1.
    #[test]
    fn children_refuse_debugger_attach() {
        let st = probe("no-ptrace");
        assert!(st.success(), "expected a non-dumpable process, got {st:?}");
    }

    /// Defense in depth beneath the allowlist: even if `socket()` were somehow
    /// reachable, the renderer's network namespace has nothing in it. This
    /// probe unshares and then enumerates interfaces, so it fails loudly if the
    /// namespace was never actually created.
    #[test]
    fn renderer_network_namespace_is_empty() {
        let st = probe("netns");
        assert!(st.success(), "expected an empty netns, got {st:?}");
    }

    /// The fork server also puts every renderer it forks in a PID namespace, so a
    /// renderer can't see or signal the broker/host by pid (defense in depth for
    /// what `kill`/`ptrace`'s absence already gives). The probe proves a forked
    /// child becomes PID 1 of a fresh namespace; it skips cleanly (exit 0) where
    /// the kernel refuses `CLONE_NEWPID`, so this passes everywhere and verifies
    /// where PID namespaces are available.
    #[test]
    fn renderer_gets_a_pid_namespace() {
        let st = probe("pidns");
        assert!(
            st.success(),
            "expected a fresh PID namespace (or a clean skip), got {st:?}"
        );
    }

    /// The fork server's filter is inherited by every renderer it forks, so a
    /// gap in it kills *renderers*, not the fork server — and surfaces as
    /// `TabCrashed`, looking nothing like a sandbox problem. This is the
    /// positive case guarding that: forking, reaping, and the
    /// `fcntl(F_DUPFD_CLOEXEC)` a forked child needs to split its endpoint
    /// before its own lockdown must all survive.
    #[test]
    fn fork_server_can_still_fork_and_reap() {
        let st = probe("forkserver-can-fork");
        assert!(st.success(), "the zygote cannot do its job under its filter: {st:?}");
    }

    /// The canary has to *detect*, not just pass. This runs it against a filter
    /// missing `set_robust_list` — one of the three syscalls that really were
    /// absent when this filter was written — and requires it to abort. A canary
    /// that only ever succeeds is indistinguishable from no canary.
    #[test]
    fn startup_canary_detects_a_missing_syscall() {
        let st = probe("forkserver-canary-gap");
        assert_eq!(st.code(), Some(1), "canary should abort with exit 1, got {st:?}");
    }

    #[test]
    fn fork_server_cannot_exec() {
        let st = probe("forkserver-no-exec");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (no exec), got {st:?}");
    }

    #[test]
    fn fork_server_cannot_open_a_socket() {
        let st = probe("forkserver-no-socket");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (no network), got {st:?}");
    }

    /// The `clone3`→`ENOSYS` + argument-filtered `clone` hardening actually
    /// bites: a plain fork works (see `can-fork`), but a `clone` into a new user
    /// namespace is trapped by the flag mask. If this exited cleanly the mask
    /// would be a silent no-op.
    #[test]
    fn fork_server_cannot_clone_into_a_new_namespace() {
        let st = probe("forkserver-no-newuser-clone");
        assert_eq!(
            st.signal(),
            Some(SIGSYS),
            "expected SIGSYS (clone flag filter), got {st:?}"
        );
    }

    /// A filesystem service's filter is the baseline *plus* `openat` — the one
    /// capability a font/storage service exists to have and a renderer denies.
    #[test]
    fn filesystem_service_may_open_files() {
        let st = probe("service-fs-openat");
        assert!(st.success(), "the fs filter should permit openat, got {st:?}");
    }

    /// ...but only that. The wider filter is still a superset of the baseline,
    /// so network is denied exactly as for a renderer — a storage service
    /// cannot phone home.
    #[test]
    fn filesystem_service_still_has_no_network() {
        let st = probe("service-fs-no-socket");
        assert_eq!(
            st.signal(),
            Some(SIGSYS),
            "fs service should have no socket, got {st:?}"
        );
    }

    /// A device service's filter permits `ioctl` — how a real audio/GPU service
    /// drives its device. (The stubs do no real work, but the filter is real.)
    #[test]
    fn device_service_may_ioctl() {
        let st = probe("service-device-ioctl");
        assert!(st.success(), "the device filter should permit ioctl, got {st:?}");
    }

    /// Landlock does what seccomp cannot: confine `openat` to specific paths. A
    /// service scoped to a directory may open files inside it but is denied
    /// (EACCES) outside — even though seccomp still permits the `openat` syscall.
    /// Skips cleanly where the kernel lacks Landlock, so it never fails an
    /// untestable host (the probe exits 0 in that case).
    #[test]
    fn landlock_scopes_a_service_to_its_directory() {
        let st = probe("service-landlock");
        assert!(
            st.success(),
            "landlock should confine openat to the ruleset, got {st:?}"
        );
    }

    /// The broker's loose Landlock: read/exec anywhere, write only beneath temp.
    /// The probe proves a write inside temp works while one outside is denied
    /// (with a control showing the outside write worked before lockdown), so the
    /// write-confinement is not a silent no-op. Skips (exit 0) without Landlock.
    #[test]
    fn broker_filesystem_writes_are_confined_to_temp() {
        let st = probe("broker-landlock");
        assert!(
            st.success(),
            "broker landlock should confine writes to temp, got {st:?}"
        );
    }

    /// The broker's deny-list seccomp filter is not a no-op: it keeps the broad
    /// surface the engine needs but `Trap`s the escalation syscalls. `ptrace` is
    /// one, so a broker that tries it is killed by `SIGSYS` exactly as a renderer
    /// reaching for a socket is — proving the trusted process lost its reach for a
    /// kernel exploit while (per the demo) still doing its job.
    #[test]
    fn broker_denies_escalation_syscalls() {
        let st = probe("broker-seccomp");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (ptrace denied), got {st:?}");
        assert!(st.code().is_none(), "should be killed, not exit");
    }

    /// The deny-list must cover the *fd-based* mount API (Linux 5.1+), not just
    /// the classic `mount`/`pivot_root`: otherwise a compromised broker reaches
    /// the same mount escape via `fsopen`+`fsmount`+`move_mount`. The probe calls
    /// `fsopen` under the broker filter and must die by `SIGSYS`.
    #[test]
    fn broker_denies_the_fd_based_mount_api() {
        let st = probe("broker-seccomp-mount");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (fsopen denied), got {st:?}");
        assert!(st.code().is_none(), "should be killed, not exit");
    }

    /// Crash reporting without a core dump or `ptrace`: a crashing content
    /// process self-captures a scrubbed report from its own signal handler, then
    /// still dies with the original signal (so the engine's crash detection is
    /// unaffected). The report carries a faulting *address*, never memory
    /// contents — leak-free even for the secret-holding broker. Checks both: the
    /// `[crash]` record on stderr, and death by `SIGSEGV`.
    #[test]
    fn a_crashing_process_self_reports_then_dies() {
        let out = Command::new(probe_bin())
            .arg("crash-report")
            .output()
            .expect("spawn sandbox-probe");
        assert_eq!(
            out.status.signal(),
            Some(SIGSEGV),
            "expected death by SIGSEGV, got {:?}",
            out.status
        );
        assert!(out.status.code().is_none(), "should be killed by the signal, not exit");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("[crash] SIGSEGV"),
            "expected a self-captured crash record, got: {err}"
        );
    }

    /// The cgroup v2 memory bound — the physical-RSS limit rlimits can't give,
    /// with a scoped OOM kill. The probe places itself in a `memory.max`-limited
    /// child cgroup and reads the ceiling back. Best-effort: where cgroup v2
    /// memory delegation isn't available (a shared scope, no `Delegate=yes`) it
    /// skips cleanly (exit 0), so this passes everywhere and *verifies* under a
    /// delegated scope. Either way a non-zero exit means the limit misbound.
    #[test]
    fn cgroup_memory_limit_binds_or_skips() {
        let st = probe("cgroup-memory-limit");
        assert!(
            st.success(),
            "cgroup memory.max should bind (or skip cleanly), got {st:?}"
        );
    }

    #[test]
    fn sealed_memfd_tile_survives_the_sandbox() {
        // The shared-memory tile producer path (memfd_create → ftruncate →
        // mmap → seal) is exactly what a confined renderer does per frame.
        let st = probe("memfd-seal");
        assert!(
            st.success(),
            "sealed-tile creation should survive the sandbox, got {st:?}"
        );
    }

    #[test]
    fn fcntl_outside_the_seal_commands_is_killed() {
        // fcntl is argument-filtered to F_ADD_SEALS/F_GET_SEALS; anything
        // else (here F_DUPFD) must be fatal.
        let st = probe("fcntl-dupfd");
        assert_eq!(st.signal(), Some(SIGSYS), "expected SIGSYS (fcntl filter), got {st:?}");
    }

    #[test]
    fn ring_buffer_survives_the_sandbox() {
        // The ring produce+consume dance (memfd + size seals, RW mapping,
        // cursor atomics) is how a confined renderer receives large bodies.
        let st = probe("ring");
        assert!(st.success(), "ring transport should survive the sandbox, got {st:?}");
    }
}
