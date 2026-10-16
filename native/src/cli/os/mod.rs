#[cfg(not(windows))]
#[path = "linux_os.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

// The local terminal of a `se shell` terminal session (raw mode, window size,
// typed bytes, output, termination requests) per platform.
#[cfg(not(windows))]
#[path = "terminal_unix.rs"]
mod terminal_platform;
#[cfg(windows)]
#[path = "terminal_windows.rs"]
mod terminal_platform;

pub(super) use terminal_platform::{LocalTerminal, TerminalOutput};

pub(super) fn same_file(left: &str, right: &str) -> std::io::Result<bool> {
    platform::same_file(left, right)
}

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    platform::local_path(path)
}

pub(super) fn validate_connection_protocol(protocol: crate::creds::Protocol) -> Result<(), String> {
    platform::validate_connection_protocol(protocol)
}

/// The Explorer context-menu entry "In Smart Explorer öffnen"; `None` where
/// the platform has no such entry (it exists only on Windows).
pub(super) fn context_menu_enabled() -> Option<bool> {
    platform::context_menu_enabled()
}

/// Registers the entry for the app executable `app`, or removes it.
pub(super) fn set_context_menu(on: bool, app: &std::path::Path) -> Result<(), String> {
    platform::set_context_menu(on, app)
}

/// Prints `prompt` to stderr and reads one line from the terminal without
/// echoing it. Ctrl+C restores the terminal and exits with status 130.
pub(super) fn read_hidden_line(prompt: &str) -> Result<String, String> {
    platform::read_hidden_line(prompt)
}

/// What the local terminal reports while a terminal session runs.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LocalEvent {
    /// Bytes the user typed, unchanged (on Windows UTF-8, with the console's
    /// virtual terminal sequences for special keys).
    Input(Vec<u8>),
    /// The window of the terminal now has this many columns and rows.
    Resized(u16, u16),
    /// Standard input ended: the terminal is gone.
    InputClosed,
    /// The process was asked to end (a signal, a console control event); the
    /// value is the exit code to end with.
    Terminate(i32),
}

/// Whether this terminal can run a raw-mode session; the terminal is left as
/// it was. The error tells why not.
pub(super) fn check_raw_mode() -> Result<(), String> {
    terminal_platform::check_raw_mode()
}

/// Columns and rows of the local terminal window, when it reports them.
pub(super) fn window_size() -> Option<(u16, u16)> {
    terminal_platform::window_size()
}
