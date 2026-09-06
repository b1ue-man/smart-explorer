//! Exact-purpose admission for the consented, separate storage-analysis window.
use std::ffi::OsString;

pub const ANALYSIS_ADMIN_FLAG: &str = "--storage-analysis-admin";

#[derive(Debug, PartialEq, Eq)]
pub struct AnalysisStartup {
    pub root: String,
    pub image_sha256: String,
}

/// Only drive-absolute paths; never a device namespace, UNC, ADS, relative path,
/// wildcard or additional command. This is also checked by the Windows adapter.
pub(crate) fn validate_local_root(root: &str) -> Result<(), String> {
    let normalized = root.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1..3] != *b":/"
        || normalized[3..].chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
        || normalized[3..].split('/').any(|part| part == "." || part == "..")
    {
        return Err("Administrator-Analyse benötigt einen absoluten lokalen Laufwerkspfad".into());
    }
    Ok(())
}

pub fn parse_analysis_startup(args: &[OsString]) -> Result<Option<AnalysisStartup>, String> {
    if !args.iter().any(|arg| arg.to_string_lossy().starts_with(ANALYSIS_ADMIN_FLAG)) {
        return Ok(None);
    }
    if args.len() != 4 || args[0] != ANALYSIS_ADMIN_FLAG || args[2] != "--image-sha256" {
        return Err("Ungültiger Administrator-Analyseaufruf".into());
    }
    let root = args[1].to_str().ok_or("Analysepfad ist nicht darstellbar")?.to_string();
    validate_local_root(&root)?;
    let hash = args[3].to_str().ok_or("Ungültige Programm-Prüfsumme")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Ungültige Programm-Prüfsumme".into());
    }
    Ok(Some(AnalysisStartup { root, image_sha256: hash.to_ascii_lowercase() }))
}

pub fn can_request_elevation(root: &str) -> bool {
    super::os::can_request_elevation(root)
}

/// Called only after an explicit user action, on a dedicated background thread.
/// `false` means consent was canceled; the original result remains untouched.
pub fn launch_elevated_analysis(root: &str) -> Result<bool, String> {
    validate_local_root(root)?;
    super::os::launch_elevated_analysis(root)
}

pub fn verify_analysis_startup(request: &AnalysisStartup) -> Result<(), String> {
    validate_local_root(&request.root)?;
    super::os::verify_analysis_startup(request)
}
