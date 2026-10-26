//! A session-owned local control socket, independent of ordered stdin traffic.
use std::{
    io::{self, Read},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use super::{RemoteExec, SetupWatch};
use crate::daemon::{exec_state::ExecState, ipc_protocol::{
    read_response, write_request, write_response, IpcRequest, IpcResponse,
}};

// The existing local Exec frame deadline also bounds the control attachment.
const ATTACH_TIMEOUT: Duration = super::LOCAL_EVENT_WRITE_TIMEOUT;

#[cfg(test)]
#[path = "batches_exec_lifetime_task_tests.rs"]
mod batches_exec_lifetime_task_tests;

#[derive(Default)]
struct OwnerState {
    claimed: bool,
    ready: bool,
    cancelled: bool,
    finished: bool,
    control: Option<TcpStream>,
}

pub(in crate::daemon) struct ExecLifetime {
    exec_id: crate::share::ExecId,
    token: String,
    input: crate::share::ShareExecInput,
    state: Mutex<OwnerState>,
    changed: Condvar,
}

impl ExecLifetime {
    fn new(exec_id: crate::share::ExecId, input: crate::share::ShareExecInput) -> io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Self { exec_id, input, token: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
            state: Mutex::new(OwnerState::default()), changed: Condvar::new() })
    }

    pub(in crate::daemon) fn token(&self) -> &str { &self.token }
    pub(in crate::daemon) fn exec_id(&self) -> &crate::share::ExecId { &self.exec_id }

    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let signal = !state.finished && !std::mem::replace(&mut state.cancelled, true);
        drop(state);
        self.changed.notify_all();
        if signal { let _ = self.input.send(crate::share::ExecClientInput::Cancel); }
    }

    fn finish(&self) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.finished = true;
        if let Some(control) = state.control.take() { let _ = control.shutdown(Shutdown::Both); }
        drop(state);
        self.changed.notify_all();
    }

    fn wait_ready(&self, setup: &SetupWatch) -> io::Result<()> {
        let deadline = Instant::now() + ATTACH_TIMEOUT;
        loop {
            setup.check()?;
            let state = self.state.lock().map_err(|_| io::Error::other("Exec-Kontrollbindung gesperrt"))?;
            if state.cancelled || state.finished {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Exec-Besitzer hat abgebrochen"));
            }
            if state.ready { return Ok(()); }
            let remaining = deadline.checked_duration_since(Instant::now()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "Exec-Kontrollbindung nicht rechtzeitig bestaetigt")
            })?;
            let (_state, _wait) = self.changed.wait_timeout(state, remaining.min(Duration::from_millis(25)))
                .map_err(|_| io::Error::other("Exec-Kontrollbindung gesperrt"))?;
        }
    }

    fn claim(&self, stream: &TcpStream) -> io::Result<()> {
        let control = stream.try_clone()?;
        let mut state = self.state.lock().map_err(|_| io::Error::other("Exec-Kontrollbindung gesperrt"))?;
        if state.claimed || state.cancelled || state.finished {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Exec-Kontrollbindung ist nicht verfuegbar"));
        }
        state.claimed = true;
        state.control = Some(control);
        Ok(())
    }

    fn ready(&self) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Exec-Kontrollbindung gesperrt"))?;
        if state.finished || state.cancelled {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Exec-Besitzer hat abgebrochen"));
        }
        state.ready = true;
        drop(state);
        self.changed.notify_all();
        Ok(())
    }
}

struct Registration {
    owner: Arc<ExecLifetime>,
    registry: Arc<ExecState>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.owner.finish();
        self.registry.remove_owner(&self.owner);
    }
}

pub(in crate::daemon) fn serve_controlled(
    mut stream: TcpStream,
    remote: RemoteExec,
    registry: Arc<ExecState>,
    setup: SetupWatch,
) -> io::Result<()> {
    let owner = Arc::new(ExecLifetime::new(remote.session.exec_id().clone(), remote.session.input())?);
    registry.register_owner(owner.clone())?;
    let _registration = Registration { owner: owner.clone(), registry: registry.clone() };
    setup.check()?;
    write_response(&mut stream, &IpcResponse::ExecControlledReady {
        exec_id: owner.exec_id().clone(), control_token: owner.token().to_owned(),
    })?;
    // The capability is already registered when Ready becomes observable. Keep
    // the setup EOF guard until the independent socket has acknowledged binding.
    owner.wait_ready(&setup)?;
    setup.handoff()?;
    super::serve(stream, remote, registry)
}

pub(in crate::daemon) fn serve_control(
    mut stream: TcpStream,
    registry: &ExecState,
    exec_id: &crate::share::ExecId,
    token: &str,
) -> io::Result<()> {
    let owner = registry.owner(exec_id, token).ok_or_else(|| {
        io::Error::new(io::ErrorKind::PermissionDenied, "Exec-Kontrollbindung stimmt nicht mit der Sitzung ueberein")
    })?;
    owner.claim(&stream)?;
    let result = (|| {
        stream.set_read_timeout(Some(ATTACH_TIMEOUT))?;
        stream.set_write_timeout(Some(ATTACH_TIMEOUT))?;
        write_response(&mut stream, &IpcResponse::ExecControlReady)?;
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
        owner.ready()?;
        // No stdin frames or application queues are consumed on this socket.
        // EOF, reset, or an unexpected control byte ends its concrete owner.
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
                Ok(_) => return Ok(()),
            }
        }
    })();
    owner.cancel();
    result
}

pub(super) fn connect_control(
    address: SocketAddr,
    token: String,
    exec_id: crate::share::ExecId,
    control_token: String,
) -> io::Result<TcpStream> {
    let mut control = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
    control.set_read_timeout(Some(ATTACH_TIMEOUT))?;
    control.set_write_timeout(Some(ATTACH_TIMEOUT))?;
    write_request(&mut control, &IpcRequest::ExecControl { token, exec_id, control_token })?;
    match read_response(&mut control)? {
        IpcResponse::ExecControlReady => {
            control.set_read_timeout(None)?;
            control.set_write_timeout(None)?;
            Ok(control)
        }
        IpcResponse::Err { msg, .. } => Err(io::Error::other(msg)),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "Worker-IPC bestaetigt keine Exec-Kontrollbindung")),
    }
}
