//! Platform parts of `se mcp`: a private protocol channel on the original
//! standard output, and how a child `se` started by the `se` tool is stopped.
#[cfg(not(windows))]
#[path = "linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

/// Returns the original standard output for the JSON-RPC stream and points
/// the process's standard output at standard error, so a stray `print!`
/// anywhere in the process cannot corrupt the protocol.
pub(super) fn protect_stdout() -> std::io::Result<std::fs::File> {
    platform::protect_stdout()
}

/// Creation options of a child `se` (Windows: no console window).
pub(super) fn configure_child(command: &mut std::process::Command) {
    platform::configure_child(command)
}

/// Asks a child to stop the way Ctrl+C would. `false` when the platform has
/// no such request for it; the caller then terminates the child.
pub(super) fn request_child_stop(child: &std::process::Child) -> bool {
    platform::request_child_stop(child)
}
