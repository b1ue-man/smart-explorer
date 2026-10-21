//! The one D-Bus connection of all portal sessions of this process.
//!
//! `ashpd` keeps its session connection in a process-wide cell, and `zbus`
//! (feature `tokio`) runs the receive loop of a connection as a task of the
//! runtime that created it. Every portal session runs on a current-thread
//! runtime of its own (`portal-session`) that ends with the session; a
//! connection created there stops answering when that runtime is gone, and
//! the next session of the process (a second start, a restart after the
//! viewer left) then waits for replies that never come. So the connection
//! is created on a thread of its own (`portal-bus`) whose runtime lives as
//! long as the process, and every session talks over a clone of it
//! (`Screencast::with_connection`, `RemoteDesktop::with_connection`), never
//! over the `ashpd` cell.
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ashpd::zbus::Connection;
use tokio::sync::{mpsc, oneshot};

type Reply = oneshot::Sender<ashpd::zbus::Result<Connection>>;

struct Shared {
    connection: Option<Connection>,
    /// Asks the bus thread for a connection; `None` before its first start.
    bus_thread: Option<mpsc::UnboundedSender<Reply>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    connection: None,
    bus_thread: None,
});

fn lock() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the thread that creates connections on its own runtime.
fn start_bus_thread() -> io::Result<mpsc::UnboundedSender<Reply>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (requests, mut inbox) = mpsc::unbounded_channel::<Reply>();
    std::thread::Builder::new()
        .name("portal-bus".into())
        .spawn(move || {
            runtime.block_on(async move {
                while let Some(reply) = inbox.recv().await {
                    let _ = reply.send(Connection::session().await);
                }
            });
        })?;
    Ok(requests)
}

/// The session bus connection of this process, created on first use and
/// kept for the life of the process.
pub(crate) async fn shared() -> Result<Connection, ashpd::Error> {
    let bus_thread = {
        let mut shared = lock();
        if let Some(connection) = &shared.connection {
            return Ok(connection.clone());
        }
        match shared.bus_thread.clone() {
            Some(bus_thread) => bus_thread,
            None => {
                let bus_thread = start_bus_thread()?;
                shared.bus_thread = Some(bus_thread.clone());
                bus_thread
            }
        }
    };
    let (reply, answer) = oneshot::channel();
    bus_thread
        .send(reply)
        .map_err(|_| ashpd::Error::NoResponse)?;
    let connection = answer.await.map_err(|_| ashpd::Error::NoResponse)??;
    // Two sessions that asked at once keep the first connection stored.
    Ok(lock().connection.get_or_insert(connection).clone())
}
