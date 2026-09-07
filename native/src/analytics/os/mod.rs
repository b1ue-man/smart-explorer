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
}

pub(super) fn read_directory(
    path: &Path,
) -> io::Result<impl Iterator<Item = io::Result<LocalEntry>>> {
    platform::read_directory(path)
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
