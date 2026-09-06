//! Build-time byte/provenance checks only. Dokany compilation belongs to the
//! remote preparation stage, never to Cargo or a cross-compilation host.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const SOURCE: &str = "f1d5de68ff459af94e309cfdd171e4b8ca2af4dd";
const ARCHIVE: &str = "f07c0a13ef426234b8707862a52f30f177528d6d34294bb0dde1620f681d266b";
const DIR_ENV: &str = "SMART_EXPLORER_DOKANY_DLL_DIR";
const SHA_ENV: &str = "SMART_EXPLORER_DOKANY_DLL_SHA256";

pub(super) fn generate() -> Result<(), String> {
    let native = env::var_os("CARGO_MANIFEST_DIR").ok_or("missing manifest directory")?;
    let native = Path::new(&native);
    let output = env::var_os("OUT_DIR").ok_or("missing build output directory")?;
    let output = Path::new(&output);
    for file in [
        "dokany-private/recipe.json",
        "dokany-private/batching.patch",
        "prepare-dokany-private.ps1",
        "build_support/private_dokany.rs",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-env-changed={DIR_ENV}");
    println!("cargo:rerun-if-env-changed={SHA_ENV}");
    let recipe_bytes = canonical_text(&native.join("dokany-private/recipe.json"))?;
    let recipe: Value = serde_json::from_slice(&recipe_bytes).map_err(|e| e.to_string())?;
    equal(text(&recipe, "source_commit")?, SOURCE, "recipe source")?;
    equal(text(&recipe, "source_archive_sha256")?, ARCHIVE, "recipe archive")?;
    if number(&recipe, "schema")? != 1 {
        return Err("unsupported recipe schema".into());
    }
    let patch_sha = hash(&canonical_text(&native.join("dokany-private/batching.patch"))?);
    equal(text(&recipe, "patch_sha256")?, &patch_sha, "recipe patch")?;
    let builder_sha = hash(&canonical_text(&native.join("prepare-dokany-private.ps1"))?);
    let recipe_sha = hash(&recipe_bytes);
    let override_dir = env::var_os(DIR_ENV);
    let expected_sha = env::var_os(SHA_ENV)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "bootstrap SHA-256 is not Unicode".to_string())
        })
        .transpose()?;
    if override_dir.is_some() != expected_sha.is_some() {
        return Err(format!("{DIR_ENV} and {SHA_ENV} must be provided together"));
    }
    let is_override = override_dir.is_some();
    let directory: PathBuf = override_dir
        .map(Into::into)
        .unwrap_or_else(|| native.join("assets/dokany-private"));
    if !directory.is_absolute() {
        return Err("DLL directory must be absolute".into());
    }
    println!("cargo:rerun-if-changed={}", directory.display());
    let dll_path = directory.join("dokan2.dll");
    let manifest_path = directory.join("manifest.json");
    if !is_override && missing(&directory)? {
        println!("cargo:warning=private Dokany payload absent; Windows mount uses the official non-batched runtime only");
        return write_generated(output, &[], "", "", &[], "");
    }
    let manifest_bytes = read_file(&manifest_path, 64 * 1024)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    if number(&manifest, "schema")? != 1 {
        return Err("unsupported payload schema".into());
    }
    for (key, expected) in [
        ("source_commit", SOURCE),
        ("source_archive_sha256", ARCHIVE),
        ("recipe_sha256", recipe_sha.as_str()),
        ("patch_sha256", patch_sha.as_str()),
        ("builder_sha256", builder_sha.as_str()),
    ] {
        equal(text(&manifest, key)?, expected, key)?;
    }
    for (key, expected) in [("library_api", 231), ("driver_protocol", 400)] {
        if number(&manifest, key)? != expected || number(&recipe, key)? != expected {
            return Err(format!("{key} mismatch"));
        }
    }
    let payload = &manifest["payload"];
    equal(text(payload, "file")?, "dokan2.dll", "payload filename")?;
    equal(text(payload, "machine")?, "AMD64", "payload architecture")?;
    let bytes = read_file(&dll_path, 32 * 1024 * 1024)?;
    verify_blob(payload, &bytes)?;
    let payload_sha = hash(&bytes);
    if let Some(expected) = expected_sha {
        validate_sha(&expected)?;
        equal(
            &payload_sha,
            &expected.to_ascii_lowercase(),
            "trusted bootstrap payload SHA",
        )?;
    }
    verify_pe_header(&bytes)?;
    let imports = strings(payload, "imports")?;
    let allowed = strings(&recipe, "allowed_imports")?;
    if imports.is_empty() || imports.iter().any(|name| !allowed.contains(name)) {
        return Err("payload imports outside the reviewed system-only dependency list".into());
    }
    let mut exports = strings(payload, "exports")?;
    let mut required = strings(&recipe, "required_exports")?;
    exports.sort_unstable();
    required.sort_unstable();
    if exports != required {
        return Err("payload exports do not match the recipe".into());
    }
    let toolchain = &manifest["toolchain"];
    equal(
        text(toolchain, "platform_toolset")?,
        "v143",
        "platform toolset",
    )?;
    equal(
        text(toolchain, "runtime_library")?,
        "MultiThreaded",
        "static CRT",
    )?;
    if version(text(toolchain, "vs_version")?)?[0] != 17 {
        return Err("payload was not prepared with the reviewed VS 2022 recipe".into());
    }
    for (key, minimum) in [
        ("msvc_version", "minimum_msvc_version"),
        ("sdk_version", "minimum_sdk_version"),
    ] {
        if version(text(toolchain, key)?)? < version(text(&recipe, minimum)?)? {
            return Err(format!("{key} is below the recipe minimum"));
        }
    }
    let source_package = &manifest["source_package"];
    equal(
        text(source_package, "file")?,
        "corresponding-source.zip",
        "source package filename",
    )?;
    let source_path = directory.join("corresponding-source.zip");
    println!("cargo:rerun-if-changed={}", source_path.display());
    let source_bytes = read_file(&source_path, 32 * 1024 * 1024)?;
    verify_blob(source_package, &source_bytes)?;
    write_generated(
        output,
        &bytes,
        &payload_sha,
        SOURCE,
        &source_bytes,
        &hash(&source_bytes),
    )
}

fn write_generated(
    output: &Path,
    bytes: &[u8],
    sha: &str,
    source: &str,
    archive: &[u8],
    archive_sha: &str,
) -> Result<(), String> {
    let initializer = if bytes.is_empty() {
        "&[]"
    } else {
        fs::write(output.join("private_dokany.dll"), bytes).map_err(|e| e.to_string())?;
        "include_bytes!(concat!(env!(\"OUT_DIR\"), \"/private_dokany.dll\"))"
    };
    let archive_initializer = if archive.is_empty() {
        "&[]"
    } else {
        fs::write(output.join("private_dokany_source.zip"), archive).map_err(|e| e.to_string())?;
        "include_bytes!(concat!(env!(\"OUT_DIR\"), \"/private_dokany_source.zip\"))"
    };
    fs::write(
        output.join("private_dokany.rs"),
        format!(
            "pub(super) const BUNDLED_DOKANY_BYTES: &[u8] = {initializer};\n\
         pub(super) const BUNDLED_DOKANY_SHA256: &str = {sha:?};\n\
         pub(super) const BUNDLED_DOKANY_SOURCE: &str = {source:?};\n\
         pub(super) const BUNDLED_DOKANY_SOURCE_ARCHIVE: &[u8] = {archive_initializer};\n\
         pub(super) const BUNDLED_DOKANY_SOURCE_SHA256: &str = {archive_sha:?};\n"
        ),
    )
    .map_err(|e| e.to_string())
}

fn missing(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
    }
}

fn read_file(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    // Release automation additionally verifies that approved inputs are tracked
    // and unchanged. Build-time checks do not claim hostile concurrent path safety.
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!("linked build input rejected: {}", ancestor.display()));
        }
    }
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        return Err(format!("invalid build input size/type: {}", path.display()));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != metadata.len() {
        return Err("build input changed while reading".into());
    }
    Ok(bytes)
}

fn canonical_text(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = read_file(path, 256 * 1024)?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    if text.starts_with('\u{feff}') {
        return Err("build recipe text must not have a BOM".into());
    }
    Ok(text.replace("\r\n", "\n").into_bytes())
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn text<'a>(object: &'a Value, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string {key}"))
}

fn number(object: &Value, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing integer {key}"))
}

fn strings<'a>(object: &'a Value, key: &str) -> Result<Vec<&'a str>, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing array {key}"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("invalid string in {key}"))
        })
        .collect()
}

fn equal(actual: &str, expected: &str, label: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label} mismatch"))
    }
}

fn validate_sha(sha: &str) -> Result<(), String> {
    if sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("invalid SHA-256".into())
    }
}

fn verify_blob(record: &Value, bytes: &[u8]) -> Result<(), String> {
    let expected = text(record, "sha256")?;
    validate_sha(expected)?;
    if number(record, "size")? != bytes.len() as u64 {
        return Err("artifact length mismatch".into());
    }
    equal(&hash(bytes), expected, "artifact SHA")
}

fn version(value: &str) -> Result<[u32; 4], String> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 4 {
        return Err("invalid toolchain version".into());
    }
    let mut result = [0; 4];
    for (index, part) in parts.iter().enumerate() {
        result[index] = part.parse().map_err(|_| "invalid toolchain version")?;
    }
    Ok(result)
}

fn verify_pe_header(bytes: &[u8]) -> Result<(), String> {
    let fail = || "private DLL is not an AMD64 PE32+ DLL".to_string();
    let offset = bytes.get(0x3c..0x40).ok_or_else(fail)?;
    let offset = u32::from_le_bytes(offset.try_into().map_err(|_| fail())?) as usize;
    let end = offset.checked_add(26).ok_or_else(fail)?;
    let header = bytes.get(offset..end).ok_or_else(fail)?;
    if bytes.get(..2) != Some(b"MZ")
        || &header[..4] != b"PE\0\0"
        || u16::from_le_bytes([header[4], header[5]]) != 0x8664
        || u16::from_le_bytes([header[22], header[23]]) & 0x2000 == 0
        || u16::from_le_bytes([header[24], header[25]]) != 0x20b
    {
        return Err(fail());
    }
    Ok(())
}
