//! Cancellation exists before TLS, route admission and the first application byte.
use std::{io, sync::{Arc, Mutex}};
use iroh::endpoint::{ApplicationPathUpdate, Connection, VarInt, WeakConnectionHandle};

type Probe = Arc<dyn Fn() -> io::Result<bool> + Send + Sync>;

#[derive(Default)]
struct State { cancelled: bool, connection: Option<WeakConnectionHandle>, probe: Option<Probe> }

#[derive(Default)]
pub(crate) struct RouteCancellation { state: Mutex<State>, changed: tokio::sync::Notify }

impl RouteCancellation {
    /// The optional OS adapter observes local EOF/Cancel without consuming input.
    pub(crate) fn set_probe(&self, probe: Option<Probe>) -> io::Result<()> {
        self.state.lock().map_err(|_| io::Error::other("Aufbauabbruch gesperrt"))?.probe = probe;
        Ok(())
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        let (cancelled, probe) = self.state.lock().map(|state| (state.cancelled, state.probe.clone()))
            .map_err(|_| io::Error::other("Aufbauabbruch gesperrt"))?;
        if cancelled { return Err(cancelled_error()); }
        if let Some(probe) = probe {
            match probe() {
                Ok(false) => {}
                Ok(true) => { self.cancel(); return Err(cancelled_error()); }
                Err(error) => { self.cancel(); return Err(error); }
            }
        }
        let state = self.state.lock().map_err(|_| io::Error::other("Aufbauabbruch gesperrt"))?;
        if state.cancelled { Err(cancelled_error()) } else { Ok(()) }
    }

    pub(crate) fn bind(&self, connection: &Connection) -> io::Result<()> {
        if let Err(error) = self.check() { stop(connection); return Err(error); }
        let mut state = self.state.lock().map_err(|_| io::Error::other("Aufbauabbruch gesperrt"))?;
        if state.cancelled { stop(connection); return Err(cancelled_error()); }
        state.connection = Some(connection.weak_handle());
        Ok(())
    }

    /// The serialized payload gate is revoked before cancellation is observable.
    /// Already accepted network sends remain in flight; they cannot be recalled.
    pub(crate) fn cancel(&self) {
        let mut state = match self.state.lock() { Ok(state) => state, Err(poison) => poison.into_inner() };
        state.cancelled = true;
        if let Some(connection) = state.connection.as_ref().and_then(WeakConnectionHandle::upgrade) { stop(&connection); }
        drop(state);
        self.changed.notify_waiters();
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().map_or(true, |state| state.cancelled) { return; }
            changed.await;
        }
    }
}

fn stop(connection: &Connection) {
    let generation = connection.application_path_policy().generation;
    let _ = connection.update_application_path_policy(ApplicationPathUpdate::Block(generation));
    connection.close(VarInt::from_u32(0x4558), b"Exec setup cancelled");
    super::route_monitor::cancelled(connection);
}

fn cancelled_error() -> io::Error { io::Error::new(io::ErrorKind::Interrupted, "Exec-Verbindungsaufbau abgebrochen") }
