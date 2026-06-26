//! Standalone build of `se-agent` (see ../Cargo.toml). The logic lives in the
//! shared, dependency-free `agent_proto` module — included here so the app and
//! this minimal crate use one definition.

#[path = "../../native/src/agent_proto/mod.rs"]
mod agent_proto;

use std::io::Write;

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    match arg.as_str() {
        "--version" | "-V" => {
            println!(
                "proto={} ver={}",
                agent_proto::PROTO_VERSION,
                env!("CARGO_PKG_VERSION")
            );
        }
        "--serve" | "" => {
            // The serve loop dispatches requests on worker threads that all
            // write through the (mutex-guarded) sink, so the writer must be
            // `Send + 'static` → hand it the owned `Stdout` (locks per write
            // internally) rather than a non-Send `StdoutLock`.
            let stdin = std::io::stdin();
            if let Err(e) = agent_proto::serve(stdin.lock(), std::io::stdout()) {
                let _ = writeln!(std::io::stderr(), "se-agent: {e}");
                std::process::exit(1);
            }
        }
        other => {
            let _ = writeln!(std::io::stderr(), "se-agent: unknown argument {other:?}");
            std::process::exit(2);
        }
    }
}
