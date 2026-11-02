//! Namespace publication is explicit; tempfile::persist supplies no write-through.
use std::{fs::File, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
fn move_name(source: &Path, target: &Path, replace: bool) -> Result<(), String> {
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // Microsoft MoveFileExW: WRITE_THROUGH does not return until the move is
    // on disk. Same-volume names are used; no copy/delete fallback is enabled.
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), flags) } == 0 {
        return Err(format!(
            "runtime_namespace_publication_failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
pub(in crate::mcp_launcher) fn publish_staged(
    staged: tempfile::NamedTempFile,
    target: &Path,
    replace: bool,
) -> Result<(), String> {
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    let (file, source) = staged.into_parts();
    drop(file);
    move_name(&source, target, replace)
}
pub(in crate::mcp_launcher) fn create_directory(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("runtime_directory_parent_missing")?;
    let temporary = tempfile::Builder::new()
        .prefix(".mcp-directory-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    super::protect(temporary.path(), true)?;
    move_name(temporary.path(), path, false)
}
pub(in crate::mcp_launcher) fn rename_durable(source: &Path, target: &Path) -> Result<(), String> {
    move_name(source, target, false)
}
pub(in crate::mcp_launcher) fn running_image() -> Result<File, String> {
    // Windows cannot replace the mapped executable. This read-only-share pin
    // additionally excludes writes/deletes while its bytes are captured.
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
