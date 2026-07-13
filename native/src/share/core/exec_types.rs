use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io;

pub(crate) const EXEC_PROTOCOL_VERSION: u16 = 1;
pub(crate) const EXEC_CAPABILITY: &str = "exec_stream_v1";
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExecCommand {
    Argv { program: String, args: Vec<String> },
    Shell { command: String },
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
}

impl ExecStart {
    pub fn validate(&self) -> io::Result<()> {
        ExecId::parse(self.exec_id.as_str())?;
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

    pub fn digest(&self) -> io::Result<String> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(super::core::eio)?;
        Ok(super::core::hex(&Sha256::digest(encoded)))
    }

    pub fn display_program(&self) -> &str {
        match &self.command {
            ExecCommand::Argv { program, .. } => program,
            ExecCommand::Shell { .. } => "<shell>",
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
}
