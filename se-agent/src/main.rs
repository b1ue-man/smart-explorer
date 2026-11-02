//! Standalone build of `se-agent` (see ../Cargo.toml). The logic lives in the
//! shared, dependency-free `agent_proto` module — included here so the app and
//! this minimal crate use one definition.

#[path = "../../native/crates/storage/src/agent_proto/mod.rs"]
mod agent_proto;
// What the shared protocol code takes from the app crates by path
// (`crate::host`, `crate::vfs`, `crate::support_dirs`); without it this crate
// does not build.
mod host;
mod support_dirs;
mod vfs;

use std::io::Write;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let arg = arguments.next().unwrap_or_default();
    match arg.as_str() {
        "--version" | "-V" => {
            println!(
                "proto={} ver={}",
                agent_proto::PROTO_VERSION,
                env!("CARGO_PKG_VERSION")
            );
        }
        "--serve-root" => {
            let Some(root) = arguments.next() else {
                let _ = writeln!(std::io::stderr(), "se-agent: --serve-root needs one path");
                std::process::exit(2);
            };
            if arguments.next().is_some() {
                let _ = writeln!(std::io::stderr(), "se-agent: unexpected extra argument");
                std::process::exit(2);
            }
            if let Err(error) = agent_proto::restrict_filesystem(std::path::Path::new(&root)) {
                let _ = writeln!(
                    std::io::stderr(),
                    "se-agent: secure root confinement unavailable: {error}"
                );
                std::process::exit(1);
            }
            // The confinement allows the root only: a cache folder outside it
            // cannot be written (or read), so the digests stay in memory.
            if !support_dirs::cache_is_inside(std::path::Path::new(&root)) {
                agent_proto::hash_cache::configure(None);
            }
            serve();
        }
        "--serve" | "" => serve(),
        other => {
            let _ = writeln!(std::io::stderr(), "se-agent: unknown argument {other:?}");
            std::process::exit(2);
        }
    }
}

fn serve() {
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
