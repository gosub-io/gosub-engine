//! Child binary for the sandbox enforcement probes.

fn main() {
    let Some(probe) = std::env::args().nth(1) else {
        eprintln!("usage: sandbox-probe <probe|list>");
        std::process::exit(2);
    };
    gosub_sandbox::selftest::run(&probe);
}
