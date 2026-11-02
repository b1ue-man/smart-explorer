// Build script of the core crate (plan docs/plaene/2026-11-01-idealzustand,
// D13): the Rust crate license notices the `licenses` module embeds.
use std::env;
use std::path::{Path, PathBuf};

#[path = "../build_support/deflate.rs"]
mod deflate;
use deflate::write_deflated;

/// Directory and file of the Rust crate license notices (plan C4), relative
/// to this package (native/crates/core). `native/rust-licenses.ps1` generates
/// the file.
const CRATE_LICENSES_DIR: &str = "../../../third-party/rust-crates";
const CRATE_LICENSES_FILE: &str = "../../../third-party/rust-crates/THIRD-PARTY-LICENSES.txt";

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    embed_crate_licenses(&out_dir);
}

/// `src/licenses/mod.rs` embeds the license file under the cfg
/// `se_crate_licenses`, which is set only when the file exists. A development
/// build without it (before the task suite generated it) still compiles and
/// every surface shows the marked notice "Lizenzverzeichnis fehlt in diesem
/// Entwicklungsbau"; the release wrapper refuses to build without the checked
/// file.
fn embed_crate_licenses(out_dir: &Path) {
    println!("cargo:rustc-check-cfg=cfg(se_crate_licenses)");
    // The directory entry also notices the file when it appears later.
    println!("cargo:rerun-if-changed={CRATE_LICENSES_DIR}");
    if Path::new(CRATE_LICENSES_FILE).is_file() {
        println!("cargo:rerun-if-changed={CRATE_LICENSES_FILE}");
        let text = std::fs::read(CRATE_LICENSES_FILE)
            .unwrap_or_else(|error| panic!("{CRATE_LICENSES_FILE}: {error}"));
        // The program reads it as text, as `include_str!` did.
        std::str::from_utf8(&text)
            .unwrap_or_else(|error| panic!("{CRATE_LICENSES_FILE} is no UTF-8: {error}"));
        write_deflated(&text, &out_dir.join("THIRD-PARTY-LICENSES.txt.deflate"));
        println!("cargo:rustc-cfg=se_crate_licenses");
    } else {
        println!(
            "cargo:warning=third-party/rust-crates/THIRD-PARTY-LICENSES.txt fehlt: dieser \
             Entwicklungsbau enthält statt der Lizenztexte nur den Hinweis \
             \"Lizenzverzeichnis fehlt in diesem Entwicklungsbau\""
        );
    }
}
