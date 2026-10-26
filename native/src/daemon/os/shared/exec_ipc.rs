use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ipc_host::ShareHost;
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};

const MAX_DATA_BYTES: usize = 64 * 1024;
const MAX_CONTROL_BYTES: usize = 64 * 1024;
const MAX_RESIZE_BYTES: usize = 256;
const LOCAL_EVENT_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

const INPUT_STDIN: u8 = 1;
const INPUT_EOF: u8 = 2;
const INPUT_CANCEL: u8 = 3;
const INPUT_RESIZE: u8 = 4;
const EVENT_AUTHORIZED: u8 = 16;
const EVENT_STARTED: u8 = 17;
const EVENT_STDOUT: u8 = 18;
const EVENT_STDERR: u8 = 19;
const EVENT_TERMINAL: u8 = 20;
const EVENT_FAILED: u8 = 21;

pub struct ExecIpcSession {
    pub exec_id: crate::share::ExecId,
    input: ExecIpcInput,
    events: TcpStream,
    finished: bool,
}

pub struct ExecIpcInput {
    stream: Arc<Mutex<TcpStream>>,
    control: Arc<TcpStream>,
}

pub(super) struct RemoteExec {
    pub(super) session: crate::share::ShareExecSession,
    pub(super) view: crate::share::ExecJobView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecIpcEvent {
    Authorized(crate::share::ExecProviderStatus),
    Started,
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Terminal(crate::share::ExecTerminal),
    Failed(ExecIpcFailure),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecIpcFailure {
    pub kind: String,
    pub code: String,
    pub message: String,
    pub start_may_have_been_sent: bool,
}

impl ExecIpcSession {
    pub fn take_input(&self) -> io::Result<ExecIpcInput> {
        Ok(self.input.clone())
    }

    pub fn next_event(&mut self) -> io::Result<ExecIpcEvent> {
        let (tag, payload) = read_frame(&mut self.events, MAX_CONTROL_BYTES)?;
        match tag {
            EVENT_AUTHORIZED => decode(&payload).map(ExecIpcEvent::Authorized),
            EVENT_STARTED if payload.is_empty() => Ok(ExecIpcEvent::Started),
            EVENT_STDOUT if payload.len() <= MAX_DATA_BYTES => Ok(ExecIpcEvent::Stdout(payload)),
            EVENT_STDERR if payload.len() <= MAX_DATA_BYTES => Ok(ExecIpcEvent::Stderr(payload)),
            EVENT_TERMINAL => {
                let terminal = decode(&payload)?;
                self.finished = true;
                Ok(ExecIpcEvent::Terminal(terminal))
            }
            EVENT_FAILED => {
                let failure = decode(&payload)?;
                self.finished = true;
                Ok(ExecIpcEvent::Failed(failure))
            }
            _ => Err(invalid("invalid local Exec event frame")),
        }
    }
}

impl Drop for ExecIpcSession {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.input.cancel();
            // Input clones may still be held by a blocked stdin producer.
            // Shutdown releases that producer without acquiring its mutex.
            let _ = self.events.shutdown(Shutdown::Both);
        }
    }
}

impl Clone for ExecIpcInput {
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.clone(),
            control: self.control.clone(),
        }
    }
}

impl ExecIpcInput {
    pub fn stdin(&self, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(MAX_DATA_BYTES) {
            self.write(INPUT_STDIN, chunk)?;
        }
        Ok(())
    }

    pub fn eof(&self) -> io::Result<()> {
        self.write(INPUT_EOF, &[])
    }

    pub fn cancel(&self) -> io::Result<()> {
        match self.control.shutdown(Shutdown::Both) {
            Err(error) if error.kind() == io::ErrorKind::NotConnected => Ok(()),
            result => result,
        }
    }

    /// New window size of a terminal start; any other start drops it.
    pub fn resize(&self, size: crate::share::ExecTerminalSize) -> io::Result<()> {
        size.validate()?;
        let payload = serde_json::to_vec(&size).map_err(io::Error::other)?;
        self.write(INPUT_RESIZE, &payload)
    }

    fn write(&self, tag: u8, payload: &[u8]) -> io::Result<()> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|_| io::Error::other("local Exec input lock poisoned"))?;
        write_frame(&mut stream, tag, payload)
    }
}

pub fn connect(
    target: crate::share::PeerOpenTarget,
    start: crate::share::ExecStart,
) -> Result<ExecIpcSession, String> {
    super::ipc_client::ensure_worker_ready()?;
    let token = super::ipc_storage::read_token()
        .map_err(|error| format!("Background-Worker Token: {error}"))?;
    let addr = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))?;
    set_stream_timeout(&stream, Some(Duration::from_secs(20)));
    write_request(
        &mut stream,
        &IpcRequest::ExecStreamControlled {
            token: token.clone(),
            target,
            start,
        },
    )
    .map_err(|error| error.to_string())?;
    let (exec_id, control_token) = match read_response(&mut stream).map_err(|error| {
        format!("Worker-IPC fuer unabhaengigen Exec-Abbruch inkompatibel oder getrennt: {error}")
    })? {
        IpcResponse::ExecControlledReady {
            exec_id,
            control_token,
        } => (exec_id, control_token),
        IpcResponse::Err { msg, .. } => return Err(msg),
        _ => return Err(
            "Worker-IPC unterstuetzt keinen unabhaengigen Exec-Abbruch; Neuverbindung erforderlich"
                .into(),
        ),
    };
    let control = lifetime::connect_control(addr, token, exec_id.clone(), control_token)
        .map_err(|error| format!("Exec-Kontrollbindung fehlgeschlagen: {error}"))?;
    set_stream_timeout(&stream, None);
    let events = stream.try_clone().map_err(|error| error.to_string())?;
    Ok(ExecIpcSession {
        exec_id,
        input: ExecIpcInput {
            stream: Arc::new(Mutex::new(stream)),
            control: Arc::new(control),
        },
        events,
        finished: false,
    })
}

#[path = "exec_connect.rs"]
mod connect;
pub(super) use connect::SetupWatch;

#[path = "exec_lifetime.rs"]
mod lifetime;
pub(super) use lifetime::{serve_control, serve_controlled, ExecLifetime};

pub(super) fn start_remote(
    host: &ShareHost,
    target: crate::share::PeerOpenTarget,
    start: crate::share::ExecStart,
    retainer: Option<Arc<dyn Send + Sync>>,
) -> Result<RemoteExec, String> {
    start_remote_cancellable(
        host,
        target,
        start,
        retainer,
        Arc::new(crate::share::route_cancellation::RouteCancellation::default()),
        Instant::now() + Duration::from_secs(45),
    )
}

pub(super) fn start_remote_cancellable(
    host: &ShareHost,
    target: crate::share::PeerOpenTarget,
    start: crate::share::ExecStart,
    retainer: Option<Arc<dyn Send + Sync>>,
    cancellation: Arc<crate::share::route_cancellation::RouteCancellation>,
    deadline: Instant,
) -> Result<RemoteExec, String> {
    connect::start_remote(host, target, start, retainer, cancellation, deadline)
}

pub(super) fn serve(
    stream: TcpStream,
    remote: RemoteExec,
    state: std::sync::Arc<super::exec_state::ExecState>,
) -> io::Result<()> {
    let mut session = Some(remote.session);
    let exec_id = session
        .as_ref()
        .ok_or_else(|| invalid("missing local Exec session"))?
        .exec_id()
        .clone();
    let mut input_stream = stream.try_clone()?;
    let input = session
        .as_ref()
        .ok_or_else(|| invalid("missing local Exec session"))?
        .input();
    let input_thread = std::thread::Builder::new()
        .name("daemon-exec-input".into())
        .spawn(move || serve_input(&mut input_stream, input))?;
    state.begin(
        remote.view,
        session
            .as_ref()
            .ok_or_else(|| invalid("missing local Exec session"))?
            .input(),
    );
    let mut output_stream = stream;
    let result = loop {
        let Some(active_session) = session.as_mut() else {
            break Err(invalid("missing local Exec session"));
        };
        let event = match active_session.next_event() {
            Ok(event) => event,
            Err(error) => break Err(error),
        };
        let forwarded = match event {
            crate::share::ExecClientEvent::Authorized {
                authorization,
                provider,
            } => {
                state.authorized(&exec_id, authorization.policy_revision);
                write_json(&mut output_stream, EVENT_AUTHORIZED, &provider)
            }
            crate::share::ExecClientEvent::Started => {
                state.started(&exec_id);
                write_frame(&mut output_stream, EVENT_STARTED, &[])
            }
            crate::share::ExecClientEvent::Stdout(bytes) => {
                write_frame(&mut output_stream, EVENT_STDOUT, &bytes)
            }
            crate::share::ExecClientEvent::Stderr(bytes) => {
                write_frame(&mut output_stream, EVENT_STDERR, &bytes)
            }
            crate::share::ExecClientEvent::Terminal(terminal) => {
                state.terminal(&terminal);
                match write_json(&mut output_stream, EVENT_TERMINAL, &terminal) {
                    Ok(()) => match session.take() {
                        Some(session) => break session.finish(),
                        None => break Err(invalid("missing local Exec session")),
                    },
                    Err(error) => break Err(error),
                }
            }
            crate::share::ExecClientEvent::Failed(error) => {
                let failure = ExecIpcFailure {
                    kind: format!("{:?}", error.kind).to_ascii_lowercase(),
                    code: error.code,
                    message: error.message,
                    start_may_have_been_sent: error.start_may_have_been_sent,
                };
                state.failed(&exec_id, &failure);
                match write_json(&mut output_stream, EVENT_FAILED, &failure) {
                    Ok(()) => match session.take() {
                        Some(session) => break session.finish(),
                        None => break Err(invalid("missing local Exec session")),
                    },
                    Err(error) => break Err(error),
                }
            }
        };
        if let Err(error) = forwarded {
            break Err(error);
        }
    };
    let _ = output_stream.shutdown(Shutdown::Both);
    if let Err(error) = &result {
        if let Some(session) = session.take() {
            let _ = session.send(crate::share::ExecClientInput::Cancel);
            let _ = session.finish();
        }
        state.failed(
            &exec_id,
            &ExecIpcFailure {
                kind: "local".into(),
                code: "local_ipc_failed".into(),
                message: error.to_string(),
                start_may_have_been_sent: true,
            },
        );
    }
    let _ = input_thread.join();
    result
}

fn target_identity(
    profiles: &crate::share::ShareProfiles,
    target: &crate::share::PeerOpenTarget,
) -> (String, String) {
    match target {
        crate::share::PeerOpenTarget::Direct { contact_id } => profiles
            .direct_contacts
            .iter()
            .find(|contact| &contact.id == contact_id)
            .map(|contact| {
                (
                    contact.remote_device_id.clone().unwrap_or_default(),
                    contact.display_name.clone(),
                )
            })
            .unwrap_or_else(|| (contact_id.clone(), contact_id.clone())),
        crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => profiles
            .rooms
            .iter()
            .find(|room| &room.id == room_id || &room.room_id == room_id)
            .and_then(|room| {
                room.members
                    .iter()
                    .find(|member| &member.device_id == device_id)
                    .map(|member| (member.device_id.clone(), member.device_name.clone()))
            })
            .unwrap_or_else(|| (device_id.clone(), device_id.clone())),
    }
}

fn serve_input(stream: &mut TcpStream, input: crate::share::ShareExecInput) -> io::Result<()> {
    loop {
        let (tag, payload) = match read_frame(stream, MAX_DATA_BYTES) {
            Ok(frame) => frame,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                let _ = input.send(crate::share::ExecClientInput::Cancel);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        input.send(client_input(tag, payload)?)?;
    }
}

/// The client input that one local input frame stands for.
fn client_input(tag: u8, payload: Vec<u8>) -> io::Result<crate::share::ExecClientInput> {
    match tag {
        INPUT_STDIN => Ok(crate::share::ExecClientInput::Stdin(payload)),
        INPUT_EOF if payload.is_empty() => Ok(crate::share::ExecClientInput::StdinEof),
        INPUT_CANCEL if payload.is_empty() => Ok(crate::share::ExecClientInput::Cancel),
        INPUT_RESIZE if payload.len() <= MAX_RESIZE_BYTES => {
            let size: crate::share::ExecTerminalSize =
                decode(&payload).map_err(|_| invalid("invalid local Exec input frame"))?;
            size.validate()
                .map_err(|_| invalid("invalid local Exec input frame"))?;
            Ok(crate::share::ExecClientInput::Resize(size))
        }
        _ => Err(invalid("invalid local Exec input frame")),
    }
}

fn write_json(stream: &mut TcpStream, tag: u8, value: &impl Serialize) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_frame(stream, tag, &payload)
}

fn write_frame(stream: &mut TcpStream, tag: u8, payload: &[u8]) -> io::Result<()> {
    let limit = if matches!(tag, INPUT_STDIN | EVENT_STDOUT | EVENT_STDERR) {
        MAX_DATA_BYTES
    } else {
        MAX_CONTROL_BYTES
    };
    if payload.len() > limit {
        return Err(invalid("local Exec frame exceeds its byte limit"));
    }
    let size = u32::try_from(payload.len()).map_err(|_| invalid("local Exec frame overflow"))?;
    let mut header = [0u8; 5];
    header[0] = tag;
    header[1..].copy_from_slice(&size.to_be_bytes());
    let deadline = Instant::now() + LOCAL_EVENT_WRITE_TIMEOUT;
    write_all_with_deadline(stream, &header, deadline, |stream, remaining| {
        stream.set_write_timeout(Some(remaining))
    })?;
    write_all_with_deadline(stream, payload, deadline, |stream, remaining| {
        stream.set_write_timeout(Some(remaining))
    })?;
    let remaining = remaining_write_time(deadline)?;
    stream.set_write_timeout(Some(remaining))?;
    stream.flush()
}

fn write_all_with_deadline<W: Write>(
    writer: &mut W,
    mut bytes: &[u8],
    deadline: Instant,
    mut prepare_write: impl FnMut(&W, Duration) -> io::Result<()>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = remaining_write_time(deadline)?;
        prepare_write(writer, remaining)?;
        match writer.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write local Exec frame",
                ))
            }
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn remaining_write_time(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "local Exec frame write timed out"))
}

fn read_frame(stream: &mut TcpStream, limit: usize) -> io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 5];
    stream.read_exact(&mut header)?;
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if size > limit {
        return Err(invalid("local Exec frame exceeds its byte limit"));
    }
    let mut payload = vec![0u8; size];
    stream.read_exact(&mut payload)?;
    Ok((header[0], payload))
}

fn decode<T: DeserializeOwned>(payload: &[u8]) -> io::Result<T> {
    serde_json::from_slice(payload).map_err(io::Error::other)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "exec_ipc_tests.rs"]
mod tests;
