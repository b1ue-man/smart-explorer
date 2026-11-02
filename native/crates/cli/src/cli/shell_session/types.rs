//! Shared types of shell sessions: command requests and their outcomes, the
//! activity the other participants of a session see, session facts, and the
//! types of terminal sessions (a login shell in a PTY, `terminal_*.rs`).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest session name; names appear in prompts, listings and entries.
const MAX_NAME_CHARS: usize = 64;
/// Largest columns and rows value of a terminal; the limit of the Share
/// protocol (`share/core/exec_terminal.rs`) and of the SSH channel.
const MAX_TERMINAL_DIMENSION: u16 = 32_767;

/// What a session runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::cli) enum SessionKind {
    /// Commands one after another with markers for exit status and working
    /// directory (the sessions of AI agents and of `se shell -T`).
    #[default]
    Commands,
    /// The login shell of the target in a pseudo-terminal: bytes in, bytes out.
    Terminal,
}

impl SessionKind {
    #[allow(dead_code)] // For the listings of the CLI and the MCP tools.
    pub(in crate::cli) fn label(self) -> &'static str {
        match self {
            Self::Commands => "commands",
            Self::Terminal => "terminal",
        }
    }
}

/// Window size of a terminal in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::cli) struct TerminalSize {
    pub(in crate::cli) cols: u16,
    pub(in crate::cli) rows: u16,
}

impl TerminalSize {
    /// Columns and rows between 1 and 32767, as the Share protocol and SSH
    /// accept them.
    pub(in crate::cli) fn validate(&self) -> Result<(), String> {
        let valid = 1..=MAX_TERMINAL_DIMENSION;
        if valid.contains(&self.cols) && valid.contains(&self.rows) {
            Ok(())
        } else {
            Err(format!(
                "terminal columns and rows must be between 1 and {MAX_TERMINAL_DIMENSION}"
            ))
        }
    }

    /// Columns and rows as the SSH `pty-req` and `window-change` carry them.
    pub(in crate::cli) fn ssh_dimensions(self) -> (u32, u32) {
        (u32::from(self.cols), u32::from(self.rows))
    }
}

impl From<TerminalSize> for crate::share::ExecTerminalSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            cols: size.cols,
            rows: size.rows,
        }
    }
}

/// What a terminal client receives from its session, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) enum TerminalEvent {
    /// Bytes of the remote terminal, unchanged; chunk borders are arbitrary.
    Output(Vec<u8>),
    /// A line from the session itself (a participant attached or detached),
    /// free of control characters.
    Notice(String),
    /// The session is over; always the last event.
    Ended {
        exit_code: Option<i32>,
        message: String,
    },
}

/// What opening a terminal session needs.
#[derive(Clone, Debug)]
pub(in crate::cli) struct TerminalOptions {
    pub(in crate::cli) name: String,
    /// The owning process as others see it, e.g. `se shell (pid 4711)`.
    pub(in crate::cli) owner: String,
    /// List the session for `se shell --attach`.
    pub(in crate::cli) attachable: bool,
    /// Working directory and environment to start with.
    pub(in crate::cli) cwd: Option<String>,
    pub(in crate::cli) env: BTreeMap<String, String>,
    /// The remote `TERM` (1 to 64 characters of `A-Za-z0-9._+-`).
    pub(in crate::cli) term: String,
    /// The size of the owner's terminal.
    pub(in crate::cli) size: TerminalSize,
}

/// Facts of a terminal session as one client sees them when it joins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // Read by the terminal front-end.
pub(in crate::cli) struct TerminalInfo {
    /// Always `terminal`; tells a client which protocol the session speaks.
    pub(in crate::cli) kind: SessionKind,
    pub(in crate::cli) id: String,
    pub(in crate::cli) name: String,
    pub(in crate::cli) target: String,
    pub(in crate::cli) owner: String,
    pub(in crate::cli) attachable: bool,
    /// The size the remote terminal has when the client joins.
    pub(in crate::cli) size: TerminalSize,
    /// This client's role.
    pub(in crate::cli) role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::cli) enum Dialect {
    /// sh or bash on Linux, Android and SSH servers.
    Posix,
    /// Windows PowerShell on Windows peers and Windows OpenSSH servers.
    PowerShell,
}

impl Dialect {
    pub(in crate::cli) fn label(self) -> &'static str {
        match self {
            Self::Posix => "posix",
            Self::PowerShell => "powershell",
        }
    }
}

/// Standard input of one command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::cli) enum StdinMode {
    /// No input (`</dev/null`).
    Null,
    /// The requester's own input follows while the command runs (POSIX only).
    Interactive,
    /// This text, then the end of input.
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::cli) struct CommandRequest {
    pub(in crate::cli) command: String,
    /// Changes the session's working directory before the command.
    #[serde(default)]
    pub(in crate::cli) cwd: Option<String>,
    /// Exported into the session before the command.
    #[serde(default)]
    pub(in crate::cli) env: BTreeMap<String, String>,
    pub(in crate::cli) stdin: StdinMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::cli) enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::cli) enum OutcomeStatus {
    /// The command ran; `exit_code` is its status (or the shell's after `exit`).
    Completed,
    /// A participant, a timeout or the owner cancelled the session.
    Cancelled,
    /// The command could not run or the session failed.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::cli) struct CommandOutcome {
    pub(in crate::cli) status: OutcomeStatus,
    pub(in crate::cli) exit_code: Option<i32>,
    /// Working directory after the command.
    pub(in crate::cli) cwd: Option<String>,
    /// The session is over (`exit`, cancel, transport end).
    pub(in crate::cli) session_ended: bool,
    pub(in crate::cli) message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::cli) enum Role {
    /// The process that opened the session; leaving closes it.
    Owner,
    /// A joined client; leaving detaches it and the session keeps running.
    Attached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::cli) struct SessionInfo {
    /// Absent in the facts sent by owners before terminal sessions existed.
    #[serde(default)]
    pub(in crate::cli) kind: SessionKind,
    pub(in crate::cli) id: String,
    pub(in crate::cli) name: String,
    /// Target selector the session was opened with.
    pub(in crate::cli) target: String,
    pub(in crate::cli) dialect: Dialect,
    /// The owning process, e.g. `se mcp (pid 4711)`.
    pub(in crate::cli) owner: String,
    pub(in crate::cli) attachable: bool,
    pub(in crate::cli) cwd: Option<String>,
    /// This client's role.
    pub(in crate::cli) role: Role,
}

/// What the other participants of a session did, as one client sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) enum Activity {
    Started {
        by: String,
        command: String,
    },
    Output {
        stream: Stream,
        bytes: Vec<u8>,
    },
    Done {
        by: String,
        exit_code: Option<i32>,
        cwd: Option<String>,
    },
    Notice(String),
    Ended {
        exit_code: Option<i32>,
        message: String,
    },
}

/// What one state change of a session produces, in order (`engine_state.rs`).
pub(super) enum Delivery {
    /// Bytes for the shell's standard input, in order.
    Write(Vec<u8>),
    /// Lossless event for one participant.
    Own { to: u64, event: OwnEvent },
    /// Lossy event for every participant except `except`.
    Activity {
        except: Option<u64>,
        activity: Activity,
    },
}

pub(super) enum OwnEvent {
    Output {
        tag: u64,
        stream: Stream,
        bytes: Vec<u8>,
    },
    Done {
        tag: u64,
        outcome: CommandOutcome,
    },
}

/// Receives the output and the end of one submitted command.
pub(in crate::cli) trait CommandSink: Send + Sync {
    fn output(&self, stream: Stream, bytes: &[u8]);
    fn finished(&self, outcome: CommandOutcome);
}

pub(in crate::cli) fn validate_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "session name {name:?} must be 1 to {MAX_NAME_CHARS} letters, digits, '-', '_' or '.'"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_name, CommandRequest, SessionKind, StdinMode, TerminalSize};

    #[test]
    fn terminal_task_sizes_are_bounded_and_convert_to_the_transports() {
        for (cols, rows, valid) in [
            (1, 1, true),
            (80, 24, true),
            (32_767, 32_767, true),
            (0, 24, false),
            (80, 0, false),
            (32_768, 24, false),
            (80, 32_768, false),
        ] {
            assert_eq!(TerminalSize { cols, rows }.validate().is_ok(), valid);
        }
        let size = TerminalSize {
            cols: 120,
            rows: 40,
        };
        assert_eq!(size.ssh_dimensions(), (120, 40));
        let shared = crate::share::ExecTerminalSize::from(size);
        assert_eq!((shared.cols, shared.rows), (120, 40));
        assert_eq!(SessionKind::default(), SessionKind::Commands);
        assert_eq!(
            serde_json::to_value(SessionKind::Terminal).unwrap(),
            "terminal"
        );
        let decoded: SessionKind = serde_json::from_str("\"commands\"").unwrap();
        assert_eq!(decoded.label(), "commands");
    }

    #[test]
    fn shell_session_task_names_and_requests_have_stable_forms() {
        assert!(validate_name("build-1.x_y").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a b").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
        let request = CommandRequest {
            command: "ls".into(),
            cwd: None,
            env: Default::default(),
            stdin: StdinMode::Text("x".into()),
        };
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["stdin"]["text"], "x");
        let decoded: CommandRequest = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, request);
    }
}
