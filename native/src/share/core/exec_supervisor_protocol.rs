use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};

use super::exec_types::ExecStart;

const MAX_CONTROL_BYTES: usize = 512 * 1024;
const MAX_DATA_BYTES: usize = 64 * 1024;

const START: u8 = 1;
const STDIN: u8 = 2;
const STDIN_EOF: u8 = 3;
const CANCEL: u8 = 4;
const STARTED: u8 = 16;
const STDOUT: u8 = 17;
const STDERR: u8 = 18;
const ROOT_EXITED: u8 = 19;
const EXITED: u8 = 20;
const ERROR: u8 = 21;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SupervisorStart {
    pub(crate) request: ExecStart,
    pub(crate) environment: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SupervisorCommand {
    Start(SupervisorStart),
    Stdin(Vec<u8>),
    StdinEof,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SupervisorExit {
    pub(crate) code: Option<i32>,
    pub(crate) signal: Option<i32>,
    pub(crate) output_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SupervisorEvent {
    Started { pid: u32 },
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    RootExited(SupervisorExit),
    Exited(SupervisorExit),
    Error(String),
}

pub(crate) fn environment_for(request: &ExecStart) -> BTreeMap<String, String> {
    let mut environment: BTreeMap<_, _> = std::env::vars()
        .filter(|(name, _)| {
            !name.is_empty() && !name.contains('=') && !is_private_environment(name)
        })
        .collect();
    for (name, value) in &request.env {
        environment.insert(name.clone(), value.clone());
    }
    environment
}

fn is_private_environment(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with("SMART_EXPLORER_")
}

pub(crate) fn send_command(writer: &mut impl Write, command: &SupervisorCommand) -> io::Result<()> {
    match command {
        SupervisorCommand::Start(start) => send_json(writer, START, start),
        SupervisorCommand::Stdin(data) => send_data(writer, STDIN, data),
        SupervisorCommand::StdinEof => send_data(writer, STDIN_EOF, &[]),
        SupervisorCommand::Cancel => send_data(writer, CANCEL, &[]),
    }
}

pub(crate) fn recv_command(reader: &mut impl Read) -> io::Result<SupervisorCommand> {
    let (tag, payload) = recv_payload(reader, MAX_CONTROL_BYTES)?;
    match tag {
        START => decode(&payload).map(SupervisorCommand::Start),
        STDIN if payload.len() <= MAX_DATA_BYTES => Ok(SupervisorCommand::Stdin(payload)),
        STDIN_EOF if payload.is_empty() => Ok(SupervisorCommand::StdinEof),
        CANCEL if payload.is_empty() => Ok(SupervisorCommand::Cancel),
        _ => Err(invalid("invalid exec supervisor command")),
    }
}

pub(crate) fn send_event(writer: &mut impl Write, event: &SupervisorEvent) -> io::Result<()> {
    match event {
        SupervisorEvent::Started { pid } => send_json(writer, STARTED, pid),
        SupervisorEvent::Stdout(data) => send_data(writer, STDOUT, data),
        SupervisorEvent::Stderr(data) => send_data(writer, STDERR, data),
        SupervisorEvent::RootExited(exit) => send_json(writer, ROOT_EXITED, exit),
        SupervisorEvent::Exited(exit) => send_json(writer, EXITED, exit),
        SupervisorEvent::Error(message) => send_json(writer, ERROR, message),
    }
}

pub(crate) fn recv_event(reader: &mut impl Read) -> io::Result<SupervisorEvent> {
    let (tag, payload) = recv_payload(reader, MAX_CONTROL_BYTES)?;
    match tag {
        STARTED => decode(&payload).map(|pid| SupervisorEvent::Started { pid }),
        STDOUT if payload.len() <= MAX_DATA_BYTES => Ok(SupervisorEvent::Stdout(payload)),
        STDERR if payload.len() <= MAX_DATA_BYTES => Ok(SupervisorEvent::Stderr(payload)),
        ROOT_EXITED => decode(&payload).map(SupervisorEvent::RootExited),
        EXITED => decode(&payload).map(SupervisorEvent::Exited),
        ERROR => decode(&payload).map(SupervisorEvent::Error),
        _ => Err(invalid("invalid exec supervisor event")),
    }
}

fn send_json(writer: &mut impl Write, tag: u8, value: &impl Serialize) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    send_data(writer, tag, &payload)
}

fn send_data(writer: &mut impl Write, tag: u8, payload: &[u8]) -> io::Result<()> {
    let limit = if matches!(tag, STDIN | STDOUT | STDERR) {
        MAX_DATA_BYTES
    } else {
        MAX_CONTROL_BYTES
    };
    if payload.len() > limit {
        return Err(invalid("exec supervisor frame exceeds its limit"));
    }
    let size = u32::try_from(payload.len()).map_err(|_| invalid("frame size overflow"))?;
    writer.write_all(&[tag])?;
    writer.write_all(&size.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

fn recv_payload(reader: &mut impl Read, limit: usize) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header)?;
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if size > limit {
        return Err(invalid("exec supervisor frame exceeds its limit"));
    }
    let mut payload = vec![0u8; size];
    reader.read_exact(&mut payload)?;
    Ok((header[0], payload))
}

fn decode<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload).map_err(io::Error::other)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::{ExecCommand, ExecId};

    #[test]
    fn binary_data_round_trips_without_json_expansion() {
        let command = SupervisorCommand::Stdin(vec![0, 1, 255]);
        let mut bytes = Vec::new();
        send_command(&mut bytes, &command).unwrap();
        assert_eq!(recv_command(&mut bytes.as_slice()).unwrap(), command);
        assert_eq!(bytes.len(), 8);
    }

    #[test]
    fn inherited_private_values_are_removed_before_explicit_overrides() {
        let request = ExecStart {
            exec_id: ExecId::parse("01".repeat(16)).unwrap(),
            command: ExecCommand::Shell {
                command: "true".into(),
            },
            cwd: None,
            env: BTreeMap::from([("SMART_EXPLORER_ALLOWED_BY_CALLER".into(), "yes".into())]),
            timeout_ms: None,
            max_output_bytes: None,
        };
        let environment = environment_for(&request);
        assert_eq!(
            environment
                .get("SMART_EXPLORER_ALLOWED_BY_CALLER")
                .map(String::as_str),
            Some("yes")
        );
    }
}
