#[cfg(not(windows))]
#[path = "linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

pub(super) fn same_file(left: &str, right: &str) -> std::io::Result<bool> {
    platform::same_file(left, right)
}

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    platform::local_path(path)
}

pub(super) fn validate_connection_protocol(protocol: crate::creds::Protocol) -> Result<(), String> {
    platform::validate_connection_protocol(protocol)
}
