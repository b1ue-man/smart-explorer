// Build script of the engine crate (plan docs/plaene/2026-11-01-idealzustand,
// D13): the private Dokany files the Windows drive host embeds. The license
// notices come from crates/core/build.rs, the SSH-agent bundles from
// crates/transfer/build.rs, the programs' Windows resources from
// native/build.rs.
use std::env;

#[path = "build_support/private_dokany.rs"]
mod private_dokany;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        private_dokany::generate()
            .unwrap_or_else(|error| panic!("private Dokany embedding contract failed: {error}"));
    }
}
