// Build script of the transfer crate (plan docs/plaene/2026-11-01-idealzustand,
// D13): the SSH-agent bundles the `agent` module deploys.
use std::env;
use std::path::{Path, PathBuf};

#[path = "../build_support/deflate.rs"]
mod deflate;
use deflate::write_deflated;

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    embed_agent_bundles(&out_dir);
}

/// The se-agent binaries the programs deploy to Linux servers
/// (`agent-bin/`, built by `build-agent-bundles.sh`).
const AGENT_BUNDLES: [&str; 2] = ["se-agent-x86_64-linux-musl", "se-agent-aarch64-linux-musl"];

/// The agent bundles go into the programs deflated (`agent/core/deploy.rs`
/// inflates the one a server needs once), as the license file does: together
/// about 1.4 MB less in every program, whose payloads must stay below
/// GitHub's 100 MiB blob limit (docs/GOTCHAS.md, "Updater").
fn embed_agent_bundles(out_dir: &Path) {
    for name in AGENT_BUNDLES {
        let source = Path::new("../../agent-bin").join(name);
        println!("cargo:rerun-if-changed={}", source.display());
        let bytes =
            std::fs::read(&source).unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        write_deflated(&bytes, &out_dir.join(format!("{name}.deflate")));
    }
}
