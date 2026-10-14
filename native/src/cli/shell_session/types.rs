//! Shared types of shell sessions: command requests and their outcomes, the
//! activity the other participants of a session see, and session facts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest session name; names appear in prompts, listings and entries.
const MAX_NAME_CHARS: usize = 64;

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
    use super::{validate_name, CommandRequest, StdinMode};

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
