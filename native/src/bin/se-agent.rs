//! `se-agent` — the headless remote helper Smart Explorer deploys over SSH.
//!
//! It runs ON THE SERVER and serves framed requests (list a dir, stat, walk a
//! whole tree for the storage analysis) over stdin/stdout, so exploration runs
//! locally where the files are and only results cross the wire. See
//! `docs/SSH_AGENT_PLAN.md`.
//!
//!   se-agent --serve     run the request/response loop on stdin/stdout
//!   se-agent --version   print "proto=<n> ver=<semver>" for the handshake
//!
//! It pulls in ONLY `agent_proto` (std + rayon) — no vfs/GUI/TLS — so it stays
//! tiny and cross-compiles to static musl for arbitrary servers.

#[path = "../agent_proto.rs"]
mod agent_proto;

use std::io::Write;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "--version" | "-V" => {
            println!("proto={} ver={}", agent_proto::PROTO_VERSION, env!("CARGO_PKG_VERSION"));
        }
        "--serve" | "" => {
            // Lock stdin/stdout once; the protocol is strictly request/response.
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let r = stdin.lock();
            let w = stdout.lock();
            if let Err(e) = agent_proto::serve(r, w) {
                let _ = writeln!(std::io::stderr(), "se-agent: {e}");
                std::process::exit(1);
            }
        }
        other => {
            let _ = writeln!(
                std::io::stderr(),
                "se-agent: unknown argument {other:?} (use --serve or --version)"
            );
            std::process::exit(2);
        }
    }
}
