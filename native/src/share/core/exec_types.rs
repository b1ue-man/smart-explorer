use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io;

use super::exec_terminal::ExecTerminalRequest;

pub(crate) const EXEC_PROTOCOL_VERSION: u16 = 2;
pub(crate) const EXEC_CAPABILITY: &str = "exec_stream_v2";
pub(crate) const MAX_EXEC_START_BYTES: usize = 128 * 1024;
pub(crate) const MAX_EXEC_DATA_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct ExecId(String);

impl ExecId {
    pub fn generate() -> io::Result<Self> {
        let bytes = super::core::random_bytes::<16>().map_err(super::core::eio)?;
        Ok(Self(super::core::hex(&bytes)))
    }

    pub fn parse(value: impl Into<String>) -> io::Result<Self> {
        let value = value.into();
        if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid(
                "exec id must contain exactly 32 hexadecimal characters",
            ));
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExecId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// What a start runs. `Login` is the interactive login shell of the host user
/// and is valid only together with a terminal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExecCommand {
    Argv { program: String, args: Vec<String> },
    Shell { command: String },
    Login,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecStart {
    pub exec_id: ExecId,
    pub command: ExecCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_bytes: Option<u64>,
    /// A start without a terminal serializes without this key, so its bytes
    /// and digest stay those of hosts and clients that predate terminals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<ExecTerminalRequest>,
}

impl ExecStart {
    pub fn validate(&self) -> io::Result<()> {
        ExecId::parse(self.exec_id.as_str())?;
        self.validate_terminal()?;
        match &self.command {
            ExecCommand::Argv { program, args } => {
                validate_os_string(program, "program")?;
                if program.trim().is_empty() {
                    return Err(invalid("remote program is empty"));
                }
                for argument in args {
                    validate_os_string(argument, "argument")?;
                }
            }
            ExecCommand::Shell { command } => {
                validate_os_string(command, "shell command")?;
                if command.trim().is_empty() {
                    return Err(invalid("remote shell command is empty"));
                }
            }
            ExecCommand::Login => {}
        }
        if let Some(cwd) = &self.cwd {
            validate_os_string(cwd, "working directory")?;
            if cwd.trim().is_empty() {
                return Err(invalid("remote working directory is empty"));
            }
        }
        for (name, value) in &self.env {
            validate_os_string(name, "environment name")?;
            validate_os_string(value, "environment value")?;
            if name.is_empty() || name.contains('=') {
                return Err(invalid("environment name is not representable by the OS"));
            }
        }
        let encoded = serde_json::to_vec(self).map_err(super::core::eio)?;
        if encoded.len() > MAX_EXEC_START_BYTES {
            return Err(invalid("remote execution start frame exceeds 128 KiB"));
        }
        Ok(())
    }

    /// The login shell runs in a terminal and only the login shell does.
    fn validate_terminal(&self) -> io::Result<()> {
        let login = matches!(self.command, ExecCommand::Login);
        match &self.terminal {
            Some(terminal) if login => terminal.validate(),
            Some(_) => Err(invalid("a terminal is only valid for the login shell")),
            None if login => Err(invalid("an interactive login shell needs a terminal")),
            None => Ok(()),
        }
    }

    pub fn digest(&self) -> io::Result<String> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(super::core::eio)?;
        Ok(super::core::hex(&Sha256::digest(encoded)))
    }

    pub fn display_program(&self) -> &str {
        match &self.command {
            ExecCommand::Argv { program, .. } => program,
            ExecCommand::Shell { .. } | ExecCommand::Login => "<shell>",
        }
    }

    pub fn effective_timeout_ms(&self) -> Option<u64> {
        self.timeout_ms.filter(|value| *value > 0)
    }

    pub fn effective_max_output_bytes(&self) -> Option<u64> {
        self.max_output_bytes.filter(|value| *value > 0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecPrincipal {
    pub relation_kind: String,
    pub relation_id: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub fingerprint: String,
    pub node_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecAuthorization {
    pub policy_revision: u64,
    pub authorization_epoch: u64,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecTerminalKind {
    Exited,
    Failed,
    TimedOut,
    Cancelled,
    Revoked,
    Disconnected,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecTerminal {
    pub exec_id: ExecId,
    pub kind: ExecTerminalKind,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub message: Option<String>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub output_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecLifecycleState {
    QueuedLocal,
    Connecting,
    Authenticating,
    Authorized,
    Starting,
    Running,
    Cancelling,
    Exited,
    Failed,
    TimedOut,
    Cancelled,
    Revoked,
    Disconnected,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecJobView {
    pub exec_id: ExecId,
    pub peer_device_id: String,
    pub peer_device_name: String,
    pub program: String,
    pub command_digest: String,
    pub state: ExecLifecycleState,
    pub policy_revision: u64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub terminal: Option<ExecTerminal>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecProviderStatus {
    pub available: bool,
    pub provider: String,
    pub detail: String,
    pub elevated: bool,
    pub user_label: String,
}

fn validate_os_string(value: &str, label: &'static str) -> io::Result<()> {
    if value.as_bytes().contains(&0) {
        return Err(invalid(match label {
            "program" => "remote program contains a NUL byte",
            "argument" => "remote argument contains a NUL byte",
            "shell command" => "remote shell command contains a NUL byte",
            "working directory" => "remote working directory contains a NUL byte",
            "environment name" => "environment name contains a NUL byte",
            _ => "environment value contains a NUL byte",
        }));
    }
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::ExecTerminalSize;

    fn start() -> ExecStart {
        ExecStart {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            command: ExecCommand::Argv {
                program: "printf".into(),
                args: vec!["".into(), "space and 'quotes' $stay-literal".into()],
            },
            cwd: Some("/tmp/a b".into()),
            env: BTreeMap::from([("UNICODE".into(), "Gruesse 🌍".into())]),
            timeout_ms: None,
            max_output_bytes: None,
            terminal: None,
        }
    }

    #[test]
    fn ids_are_exact_random_128_bit_hex_values() {
        let id = ExecId::generate().unwrap();
        assert_eq!(id.as_str().len(), 32);
        assert!(ExecId::parse(id.to_string()).is_ok());
        assert!(ExecId::parse("short").is_err());
    }

    #[test]
    fn direct_argv_preserves_literals_and_has_a_stable_digest() {
        let request = start();
        request.validate().unwrap();
        assert_eq!(request.digest().unwrap(), request.digest().unwrap());
        let decoded: ExecStart =
            serde_json::from_slice(&serde_json::to_vec(&request).unwrap()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn unset_limits_are_unlimited_and_only_os_invalid_values_are_rejected() {
        let request = start();
        assert_eq!(request.effective_timeout_ms(), None);
        assert_eq!(request.effective_max_output_bytes(), None);
        let mut invalid = request;
        invalid.env.insert("A=B".into(), "value".into());
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn zero_limits_are_canonical_unlimited_values() {
        let mut request = start();
        request.timeout_ms = Some(0);
        request.max_output_bytes = Some(0);
        assert_eq!(request.effective_timeout_ms(), None);
        assert_eq!(request.effective_max_output_bytes(), None);
    }

    fn terminal_request(term: &str, cols: u16, rows: u16) -> ExecTerminalRequest {
        ExecTerminalRequest {
            term: term.into(),
            size: ExecTerminalSize { cols, rows },
        }
    }

    fn login(terminal: Option<ExecTerminalRequest>) -> ExecStart {
        ExecStart {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            command: ExecCommand::Login,
            cwd: None,
            env: BTreeMap::new(),
            timeout_ms: None,
            max_output_bytes: None,
            terminal,
        }
    }

    #[test]
    fn terminal_task_start_without_terminal_keeps_its_bytes_and_digest() {
        let request = ExecStart {
            exec_id: ExecId::parse("00".repeat(16)).unwrap(),
            command: ExecCommand::Argv {
                program: "printf".into(),
                args: vec!["".into(), "a b".into()],
            },
            cwd: Some("/tmp/a b".into()),
            env: BTreeMap::from([("K".into(), "V".into())]),
            timeout_ms: Some(5000),
            max_output_bytes: Some(1024),
            terminal: None,
        };
        // The encoding and digest below are those of the start type before
        // terminals existed: the struct had exactly these keys in this order.
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"exec_id":"00000000000000000000000000000000","command":{"mode":"argv","program":"printf","args":["","a b"]},"cwd":"/tmp/a b","env":{"K":"V"},"timeout_ms":5000,"max_output_bytes":1024}"#
        );
        assert!(!encoded.contains("terminal"));
        assert_eq!(
            request.digest().unwrap(),
            "b6626090fc5cb1f61c514e981661139a799f40e08688115a35af93d0d349e64a"
        );
        let decoded: ExecStart = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn terminal_task_terminal_start_carries_the_key_last_and_round_trips() {
        let request = login(Some(terminal_request("xterm-256color", 80, 24)));
        request.validate().unwrap();
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            encoded,
            r#"{"exec_id":"00000000000000000000000000000000","command":{"mode":"login"},"terminal":{"term":"xterm-256color","size":{"cols":80,"rows":24}}}"#
        );
        let decoded: ExecStart = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);
        assert_ne!(
            request.digest().unwrap(),
            login(Some(terminal_request("xterm-256color", 80, 25)))
                .digest()
                .unwrap()
        );
    }

    #[test]
    fn terminal_task_size_limits_are_one_to_32767() {
        let cases = [
            (0, 24, false),
            (80, 0, false),
            (32768, 24, false),
            (80, 32768, false),
            (u16::MAX, u16::MAX, false),
            (1, 1, true),
            (80, 24, true),
            (32767, 32767, true),
        ];
        for (cols, rows, valid) in cases {
            let size = ExecTerminalSize { cols, rows };
            assert_eq!(size.validate().is_ok(), valid, "{cols}x{rows}");
            let start = login(Some(terminal_request("xterm", cols, rows)));
            assert_eq!(start.validate().is_ok(), valid, "{cols}x{rows}");
            if !valid {
                let error = size.validate().unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            }
        }
    }

    #[test]
    fn terminal_task_term_value_is_a_short_portable_name() {
        let too_long = "a".repeat(65);
        let longest = "a".repeat(64);
        for term in [
            "",
            "xterm 256",
            " xterm",
            "xterm\u{7}",
            "xterm\n",
            "xterm\0",
            "x/y",
            "x=y",
            "grüne",
            too_long.as_str(),
        ] {
            let error = login(Some(terminal_request(term, 80, 24)))
                .validate()
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{term:?}");
        }
        for term in [
            "xterm-256color",
            "linux",
            "screen.xterm-256color",
            "a+b_c.d-e",
            "X",
            longest.as_str(),
        ] {
            login(Some(terminal_request(term, 80, 24)))
                .validate()
                .unwrap_or_else(|error| panic!("{term:?}: {error}"));
        }
    }

    #[test]
    fn terminal_task_login_and_terminal_only_exist_together() {
        let error = login(None).validate().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let mut with_options = login(Some(terminal_request("xterm", 80, 24)));
        with_options.cwd = Some("/tmp".into());
        with_options.env.insert("LANG".into(), "C".into());
        with_options.timeout_ms = Some(1000);
        with_options.validate().unwrap();

        let mut argv = start();
        argv.terminal = Some(terminal_request("xterm", 80, 24));
        let error = argv.validate().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        let mut shell = start();
        shell.command = ExecCommand::Shell {
            command: "true".into(),
        };
        shell.terminal = Some(terminal_request("xterm", 80, 24));
        let error = shell.validate().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        shell.terminal = None;
        shell.validate().unwrap();
    }

    #[test]
    fn terminal_task_login_is_a_unit_mode_and_hidden_in_the_history() {
        assert_eq!(
            serde_json::to_string(&ExecCommand::Login).unwrap(),
            r#"{"mode":"login"}"#
        );
        let decoded: ExecCommand = serde_json::from_str(r#"{"mode":"login"}"#).unwrap();
        assert_eq!(decoded, ExecCommand::Login);
        let request = login(Some(terminal_request("xterm", 80, 24)));
        assert_eq!(request.display_program(), "<shell>");
        assert_eq!(
            ExecStart {
                command: ExecCommand::Shell {
                    command: "secret".into()
                },
                terminal: None,
                ..request
            }
            .display_program(),
            "<shell>"
        );
    }
}
