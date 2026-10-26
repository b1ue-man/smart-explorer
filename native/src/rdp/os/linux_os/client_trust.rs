//! Query only the installed client's native configuration-path API. No RDP
//! protocol/session API is called, and no library is redistributed.
use crate::rdp::os::shared::process::{capture, find_tool};
use std::{
    ffi::{CStr, CString},
    path::{Path, PathBuf},
};

pub(super) fn verify(client: &Path) -> Result<(), String> {
    if std::env::var_os("LD_PRELOAD").is_some() || std::env::var_os("LD_LIBRARY_PATH").is_some() {
        return Err("FreeRDP client verification cannot be bound while dynamic-loader overrides are present".into());
    }
    let client = trusted_system_path(client)?;
    let ldd = trusted_system_path(&find_tool(&["ldd"]).ok_or_else(|| "Cannot identify the installed FreeRDP client's native certificate-policy library: ldd is unavailable".to_string())?)?;
    let client = client
        .to_str()
        .ok_or_else(|| "FreeRDP client path is not UTF-8".to_string())?;
    let dependencies = capture(&ldd, &[client])?;
    let paths: Vec<_> = dependencies
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.trim().split_once(" => ")?;
            if name != "libfreerdp3.so.3" && name != "libfreerdp3.so" {
                return None;
            }
            let path = rest.split_whitespace().next()?;
            Path::new(path).is_absolute().then(|| PathBuf::from(path))
        })
        .collect();
    if paths.len() != 1 {
        return Err("The external client has no uniquely identified shared FreeRDP3 policy library; static/custom builds need a separate verified adapter".into());
    }
    let library_path = trusted_system_path(&paths[0])?;
    // SAFETY: an absolute, installed, administrator-owned client dependency is
    // selected, never a project/search-path library. Only the documented pure
    // configuration-path allocator is called, using its C ABI and BOOL=i32.
    let library = unsafe { libloading::Library::new(&library_path) }
        .map_err(|e| format!("Cannot read installed FreeRDP native policy API: {e}"))?;
    type ConfigPath = unsafe extern "C" fn(i32, *const libc::c_char) -> *mut libc::c_char;
    let file = CString::new("certificates.json").map_err(|e| e.to_string())?;
    // SAFETY: signature and symbol are documented by FreeRDP utils/helpers.h.
    let function: libloading::Symbol<'_, ConfigPath> =
        unsafe { library.get(b"freerdp_GetConfigFilePath\0") }.map_err(|_| {
            "Installed FreeRDP cannot disclose its exact system certificate-policy path".to_string()
        })?;
    // SAFETY: the filename C string lives through the call; result is malloc-owned.
    let raw = unsafe { function(1, file.as_ptr()) };
    if raw.is_null() {
        return Err("FreeRDP system certificate-policy path is unavailable".into());
    }
    // SAFETY: the documented successful return is a NUL-terminated malloc string.
    let result = unsafe { CStr::from_ptr(raw) }
        .to_str()
        .map(PathBuf::from)
        .map_err(|_| "FreeRDP certificate-policy path is not UTF-8".to_string());
    // SAFETY: this Linux library allocates the returned path with libc malloc.
    unsafe {
        libc::free(raw.cast());
    }
    let policy = result?;
    if !policy.is_absolute() {
        return Err("FreeRDP certificate-policy path is not absolute".into());
    }
    for parent in policy.ancestors().skip(1) {
        use std::os::unix::fs::MetadataExt;
        match std::fs::metadata(parent) {
            Ok(metadata) if metadata.is_dir() && metadata.uid() == 0 && metadata.mode() & 0o022 == 0 => {}
            Ok(_) => return Err("FreeRDP system certificate-policy directory is writable outside the administrator boundary".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("FreeRDP certificate-policy parent cannot be inspected: {error}")),
        }
    }
    match std::fs::symlink_metadata(&policy) {
        // This conservatively avoids all native global overrides, including
        // duplicate-key/cJSON interpretation differences and ignore:true.
        Ok(_) => Err(format!("FreeRDP system certificate policy {} is present; its overrides have no approved automatic-verification adapter", policy.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("FreeRDP system certificate policy cannot be inspected: {error}")),
    }
}

fn trusted_system_path(path: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    let path =
        std::fs::canonicalize(path).map_err(|e| format!("Native client system path: {e}"))?;
    let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Native client system path is not a file".into());
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::metadata(ancestor).map_err(|e| e.to_string())?;
        if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err("Native RDP client, policy library and their containing directories must be administrator-owned without group/world write access".into());
        }
    }
    Ok(path)
}
