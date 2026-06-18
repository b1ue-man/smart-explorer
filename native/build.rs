use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-env-changed=WINDRES");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
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
