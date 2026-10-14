//! Persistent remote shell sessions (docs/plaene/2026-10-14-analyse-id-plugin-vorrang/plan.md,
//! R4 and its attach extension). One shell runs over an existing exec stream
//! (the SSH exec channel of a saved SFTP remote or a Share peer's Exec stream)
//! and keeps its working directory, environment and shell state across
//! commands; each command ends with markers carrying its exit status and
//! working directory. The owning `se` process (`se mcp` or `se shell`) hosts
//! the session behind a loopback attach service: its own front-end and every
//! attached client use the same framed protocol, authenticated by a
//! per-session token. Attachable sessions are listed in the owner-only
//! directory `<app data>/shell-sessions/`.
//!
//! Next to this command session there is the terminal session
//! (docs/plaene/2026-10-14-echtes-terminal/plan.md, M6): the login shell of
//! the target in a pseudo-terminal, its bytes passed through unchanged, hosted
//! behind the same attach service with protocol version 2
//! (`terminal_{engine,server,client,transports}.rs`).
#[path = "shell_session/client.rs"]
mod client;
#[path = "shell_session/dialect.rs"]
mod dialect;
#[path = "shell_session/engine.rs"]
mod engine;
#[path = "shell_session/engine_state.rs"]
mod engine_state;
#[path = "shell_session/host.rs"]
mod host;
#[cfg(test)]
#[path = "shell_session/local_transport.rs"]
mod local_transport;
#[path = "shell_session/marker.rs"]
mod marker;
#[cfg(all(test, unix))]
#[path = "shell_session/pty_test_transport.rs"]
mod pty_test_transport;
#[path = "shell_session/registry.rs"]
mod registry;
#[path = "shell_session/server.rs"]
mod server;
#[path = "shell_session/terminal_client.rs"]
mod terminal_client;
#[path = "shell_session/terminal_engine.rs"]
mod terminal_engine;
#[path = "shell_session/terminal_server.rs"]
mod terminal_server;
#[cfg(test)]
#[path = "shell_session/terminal_tests.rs"]
mod terminal_tests;
#[path = "shell_session/terminal_transports.rs"]
mod terminal_transports;
#[cfg(test)]
#[path = "shell_session/tests.rs"]
mod tests;
#[path = "shell_session/transports.rs"]
mod transports;
#[path = "shell_session/types.rs"]
mod types;
#[path = "shell_session/wire.rs"]
mod wire;

pub(in crate::cli) use client::SessionClient;
pub(in crate::cli) use dialect::argv_command;
#[allow(unused_imports)] // The terminal front-end and the MCP tools use these.
pub(in crate::cli) use host::{open, open_terminal, OpenOptions, TerminalOpen};
pub(in crate::cli) use registry::{find as find_attachable, list as list_attachable};
#[allow(unused_imports)] // The terminal front-end uses it.
pub(in crate::cli) use terminal_client::TerminalClient;
pub(in crate::cli) use transports::resolve_target;
pub(in crate::cli) use types::{
    validate_name, Activity, CommandOutcome, CommandRequest, CommandSink, Dialect, OutcomeStatus,
    Role, StdinMode, Stream,
};
#[allow(unused_imports)] // The terminal front-end and the MCP tools use these.
pub(in crate::cli) use types::{
    SessionKind, TerminalEvent, TerminalInfo, TerminalOptions, TerminalSize,
};
