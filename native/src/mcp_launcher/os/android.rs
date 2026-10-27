//! Android has no local MCP host or installer. Keep wire types available while
//! refusing every desktop lifecycle operation explicitly.
use crate::mcp_launcher::{
    core::lifecycle::{ProcessIdentity, StopIdentity},
    GenerationChild,
};
use std::{fs::File, path::Path, process::Command};
pub(in crate::mcp_launcher) const CLI_NAME: &str = "se";
pub(in crate::mcp_launcher) const LAUNCHER_NAME: &str = "se-mcp-launcher";
pub(in crate::mcp_launcher) type NativeChild = std::process::Child;
fn unsupported<T>() -> Result<T, String> {
    Err("local_mcp_runtime_unsupported_on_android".into())
}
pub(in crate::mcp_launcher) fn validate_tree(_root: &Path, _path: &Path) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn open_verified(
    _root: &Path,
    _path: &Path,
    _hash: &str,
) -> Result<File, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn lock(_path: &Path, _exclusive: bool) -> Result<File, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn legacy_profile_image(_path: &Path) -> Result<File, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn legacy_update_image(_path: &Path) -> Result<Option<File>, String> {
    Ok(None)
}
pub(in crate::mcp_launcher) fn same_image_file(_file: &File, _path: &Path) -> Result<bool, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn protect(_path: &Path, _executable: bool) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn make_private_directory(_path: &Path) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn sync_directory(_path: &Path) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn process_identity(_pid: u32) -> Result<ProcessIdentity, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn process_matches(_identity: &ProcessIdentity) -> bool {
    false
}
pub(in crate::mcp_launcher) fn processes_for(_path: &Path) -> Result<Vec<ProcessIdentity>, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn uninstall_programs_idle(_root: &Path) -> Result<Vec<File>, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn retirement_root() -> Result<Option<crate::mcp_launcher::installation::RetirementRoot>, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn retirement_handoff(_root: &Path, _hash: &str, _snapshot: &str) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn removal_image_guard(_path: &Path) -> Result<Option<File>, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn stop_identity(_path: &Path) -> Result<StopIdentity, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn spawn_verified(
    _command: Command,
    _image: File,
    _lifecycle: bool,
    _capture: bool,
    _input: bool,
) -> Result<GenerationChild, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn spawn_legacy(
    _command: Command,
    _input: bool,
) -> Result<GenerationChild, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn run_stable_launcher(
    _command: Command,
    _image: File,
) -> Result<i32, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn spawn_current_daemon(
    _command: Command,
    _image: File,
) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn spawn_installation_probe(
    _command: Command,
    _image: File,
) -> Result<GenerationChild, String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn take_admission_channel(
) -> Result<Option<(File, bool, Option<File>)>, String> {
    Ok(None)
}
pub(in crate::mcp_launcher) fn acknowledge(_channel: &mut File) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn request_stop(_pid: u32) -> bool {
    false
}

pub(in crate::mcp_launcher) fn publish_staged(
    _file: tempfile::NamedTempFile,
    _target: &Path,
    _replace: bool,
) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn create_directory(_path: &Path) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn rename_durable(
    _source: &Path,
    _target: &Path,
) -> Result<(), String> {
    unsupported()
}
pub(in crate::mcp_launcher) fn running_image() -> Result<File, String> {
    unsupported()
}
