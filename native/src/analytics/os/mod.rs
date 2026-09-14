use std::{ffi::OsString, io, path::Path};

#[cfg(not(windows))]
#[path = "shared/local_directory.rs"]
mod platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryKind {
    File,
    Directory,
    Link,
    Other,
}

pub(super) struct LocalEntry {
    pub name: OsString,
    pub kind: EntryKind,
    pub size: u64,
    /// The name could not be represented as an OS path (for example a
    /// directory record carrying NUL or separator characters). The entry is
    /// still counted; a directory with such a name is not descended into.
    pub unreachable: bool,
}

pub(super) fn read_directory(
    path: &Path,
) -> io::Result<impl Iterator<Item = io::Result<LocalEntry>>> {
    platform::read_directory(path)
}

/// The exact path the scanner should open for `root`: on Windows the
/// verbatim (`\\?\`) absolute form, so reserved device names, trailing
/// dots/spaces and long paths are addressed literally; elsewhere unchanged.
pub(super) fn normalize_scan_root(root: &Path) -> std::path::PathBuf {
    platform::normalize_scan_root(root)
}

/// A path as shown to the user (verbatim prefixes stripped).
pub(super) fn display_path(path: &Path) -> String {
    platform::display_path(path)
}

pub(super) fn parallel_scan_allowed() -> bool {
    platform::parallel_scan_allowed()
}

pub(super) fn can_request_elevation(root: &str) -> bool {
    platform::can_request_elevation(root)
}
pub(super) fn launch_elevated_analysis(root: &str) -> Result<bool, String> {
    platform::launch_elevated_analysis(root)
}
pub(super) fn verify_analysis_startup(request: &super::AnalysisStartup) -> Result<(), String> {
    platform::verify_analysis_startup(request)
}
