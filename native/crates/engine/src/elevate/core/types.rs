//! What an elevated (or directly started) command is, how it ended and why
//! an elevation did not happen. Platform-free.
use std::collections::BTreeMap;
use std::fmt;

/// The program an [`ElevatedSpec`] starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElevatedCommand {
    /// A program with literal arguments (no shell of its own).
    Argv { program: String, args: Vec<String> },
    /// Windows only: `cmd.exe /D /S /C <line>`, the line handed to cmd.exe
    /// unquoted, so cmd's own quoting rules apply (as the Share host does).
    CmdLine(String),
}

/// A command to run, with its working directory and extra environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElevatedSpec {
    pub command: ElevatedCommand,
    /// Working directory; `None` keeps the starting process's own (an
    /// elevated start then runs in the system's default for the account).
    pub cwd: Option<String>,
    /// Variables set on top of the started process's environment.
    pub env: BTreeMap<String, String>,
}

impl ElevatedSpec {
    /// The command as one readable line (POSIX-style quoting of arguments
    /// that need it), for summaries and confirmation texts.
    pub fn display(&self) -> String {
        match &self.command {
            ElevatedCommand::Argv { program, args } => std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .map(readable_word)
                .collect::<Vec<_>>()
                .join(" "),
            ElevatedCommand::CmdLine(line) => format!("cmd.exe /D /S /C {line}"),
        }
    }
}

fn readable_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|ch| ch.is_alphanumeric() || "-_./:=+,@%".contains(ch));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// Through which way a command got its rights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    /// With the rights of the starting process (not elevated).
    Direct,
    /// The starting process is elevated already (administrator token or root).
    Already,
    /// Windows: UAC consent through `ShellExecuteExW` „runas“.
    Uac,
    /// Linux: `pkexec` with the polkit agent of the session.
    Polkit,
    /// Linux: `sudo -n` (no password needed for this account).
    SudoNoPassword,
}

impl Elevation {
    /// The way as the results of the MCP tools name it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Direct => "none",
            Self::Already => "already elevated",
            Self::Uac => "uac",
            Self::Polkit => "polkit",
            Self::SudoNoPassword => "sudo -n",
        }
    }

    /// Whether the command runs with administrator or root rights.
    pub fn elevated(self) -> bool {
        !matches!(self, Self::Direct)
    }
}

/// Why an elevation or a confirmation of the user did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElevateError {
    /// The user dismissed the system dialog (UAC „Nein“, polkit cancel).
    Declined,
    /// No system dialog can ask the user here: no polkit agent in this
    /// session (and no `sudo -n` right), no pkexec. The text says which.
    NoAgent(String),
    /// The system refused the account (not an administrator, sudoers,
    /// polkit „not authorized“). The text is the system's reason.
    NotAllowed(String),
    /// Anything else (start failed, helper protocol broken).
    Failed(String),
}

impl fmt::Display for ElevateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declined => {
                formatter.write_str("the user declined the elevation in the system dialog")
            }
            Self::NoAgent(why) => write!(
                formatter,
                "no system dialog can ask for the elevation here: {why}"
            ),
            Self::NotAllowed(why) => write!(formatter, "the system refused the elevation: {why}"),
            Self::Failed(why) => write!(formatter, "the elevated start failed: {why}"),
        }
    }
}

impl std::error::Error for ElevateError {}

/// How a started command ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElevatedExit {
    /// Exit status of the command.
    pub code: Option<i32>,
    /// Signal that ended it (Unix).
    pub signal: Option<i32>,
    /// The command was cancelled (`ElevatedInput::cancel`).
    pub cancelled: bool,
    /// The command could not be run or its transport broke.
    pub error: Option<String>,
}

impl ElevatedExit {
    /// The status as a shell reports it: the code, `128 + n` after signal
    /// `n`, `None` without either.
    pub fn status(&self) -> Option<i32> {
        self.code.or_else(|| self.signal.map(|signal| 128 + signal))
    }
}

/// What a started command produced; `Exited` is always the last event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElevatedEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exited(ElevatedExit),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_elevate_task_display_quotes_only_what_needs_it() {
        let spec = ElevatedSpec {
            command: ElevatedCommand::Argv {
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "ls -l /root".into(),
                    "it's".into(),
                    String::new(),
                ],
            },
            cwd: None,
            env: BTreeMap::new(),
        };
        assert_eq!(spec.display(), "/bin/sh -c 'ls -l /root' 'it'\\''s' ''");
        let cmd = ElevatedSpec {
            command: ElevatedCommand::CmdLine("dir \"C:\\Program Files\"".into()),
            ..spec
        };
        assert_eq!(cmd.display(), "cmd.exe /D /S /C dir \"C:\\Program Files\"");
    }

    #[test]
    fn shell_elevate_task_exit_status_follows_the_shell_convention() {
        let exit = ElevatedExit {
            signal: Some(9),
            ..ElevatedExit::default()
        };
        assert_eq!(exit.status(), Some(137));
        let exit = ElevatedExit {
            code: Some(3),
            signal: Some(9),
            ..ElevatedExit::default()
        };
        assert_eq!(exit.status(), Some(3));
        assert_eq!(ElevatedExit::default().status(), None);
        assert!(Elevation::Uac.elevated() && !Elevation::Direct.elevated());
        assert!(ElevateError::Declined.to_string().contains("declined"));
    }
}
