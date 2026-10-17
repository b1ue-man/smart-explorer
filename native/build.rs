use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "build_support/private_dokany.rs"]
mod private_dokany;

/// Directory and file of the Rust crate license notices (plan C4), relative
/// to this package. `native/rust-licenses.ps1` generates the file.
const CRATE_LICENSES_DIR: &str = "../third-party/rust-crates";
const CRATE_LICENSES_FILE: &str = "../third-party/rust-crates/THIRD-PARTY-LICENSES.txt";

fn main() {
    embed_crate_licenses();
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/smart-explorer-icon.ico");
    println!("cargo:rerun-if-env-changed=WINDRES");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    private_dokany::generate()
        .unwrap_or_else(|error| panic!("private Dokany embedding contract failed: {error}"));
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    let obj = out_dir.join("smart_explorer_manifest.o");
    let windres = find_windres();
    let status = Command::new(&windres)
        .arg("--input-format=rc")
        .arg("--output-format=coff")
        .arg("app.rc")
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {}", windres, e));

    if !status.success() {
        panic!("{} failed while compiling app.rc", windres);
    }

    println!("cargo:rustc-link-arg-bins={}", obj.display());
}

/// `src/licenses/mod.rs` embeds the license file under the cfg
/// `se_crate_licenses`, which is set only when the file exists. A development
/// build without it (before the task suite generated it) still compiles and
/// every surface shows the marked notice "Lizenzverzeichnis fehlt in diesem
/// Entwicklungsbau"; the release wrapper refuses to build without the checked
/// file.
fn embed_crate_licenses() {
    println!("cargo:rustc-check-cfg=cfg(se_crate_licenses)");
    // The directory entry also notices the file when it appears later.
    println!("cargo:rerun-if-changed={CRATE_LICENSES_DIR}");
    if Path::new(CRATE_LICENSES_FILE).is_file() {
        println!("cargo:rerun-if-changed={CRATE_LICENSES_FILE}");
        println!("cargo:rustc-cfg=se_crate_licenses");
    } else {
        println!(
            "cargo:warning=third-party/rust-crates/THIRD-PARTY-LICENSES.txt fehlt: dieser \
             Entwicklungsbau enthält statt der Lizenztexte nur den Hinweis \
             \"Lizenzverzeichnis fehlt in diesem Entwicklungsbau\""
        );
    }
}

fn find_windres() -> String {
    if let Some(path) = env::var_os("WINDRES") {
        return path.to_string_lossy().into_owned();
    }

    for candidate in ["x86_64-w64-mingw32-windres", "windres"] {
        if Command::new(candidate).arg("--version").output().is_ok() {
            return candidate.to_string();
        }
    }

    "windres".to_string()
}
