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
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    embed_crate_licenses(&out_dir);
    embed_agent_bundles(&out_dir);
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

/// The se-agent binaries the programs deploy to Linux servers
/// (`agent-bin/`, built by `build-agent-bundles.sh`).
const AGENT_BUNDLES: [&str; 2] = ["se-agent-x86_64-linux-musl", "se-agent-aarch64-linux-musl"];

/// The agent bundles go into the programs deflated (`agent/core/deploy.rs`
/// inflates the one a server needs once), as the license file does: together
/// about 1.4 MB less in every program, whose payloads must stay below
/// GitHub's 100 MiB blob limit (docs/GOTCHAS.md, "Updater").
fn embed_agent_bundles(out_dir: &Path) {
    for name in AGENT_BUNDLES {
        let source = Path::new("agent-bin").join(name);
        println!("cargo:rerun-if-changed={}", source.display());
        let bytes =
            std::fs::read(&source).unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        write_deflated(&bytes, &out_dir.join(format!("{name}.deflate")));
    }
}

/// Writes `bytes` deflated (raw deflate, best compression) to `target`, after
/// proving that the program's inflation gives them back unchanged.
fn write_deflated(bytes: &[u8], target: &Path) {
    use std::io::{Read, Write};
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(bytes)
        .unwrap_or_else(|error| io_failed(target, error));
    let packed = encoder
        .finish()
        .unwrap_or_else(|error| io_failed(target, error));
    let mut back = Vec::with_capacity(bytes.len());
    flate2::read::DeflateDecoder::new(packed.as_slice())
        .read_to_end(&mut back)
        .unwrap_or_else(|error| io_failed(target, error));
    assert!(back == bytes, "deflate round trip of {}", target.display());
    std::fs::write(target, packed).unwrap_or_else(|error| io_failed(target, error));
}

fn io_failed(target: &Path, error: std::io::Error) -> ! {
    panic!("{}: {error}", target.display())
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
