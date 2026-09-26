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

/// Prints `prompt` to stderr and reads one line from the terminal without
/// echoing it. Ctrl+C restores the terminal and exits with status 130.
pub(super) fn read_hidden_line(prompt: &str) -> Result<String, String> {
    platform::read_hidden_line(prompt)
}
