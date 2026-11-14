//! Two bounded copy directions. EOF shuts down only the opposite writer;
//! the remaining direction drains independently. Route pauses preserve the
//! same pending operation and pause its active idle budget (P28).
//!
//! Plan 2026-11-14, E7 and E3: the pump subscribes to the route status once
//! (`PumpRoute`), the idle budget is one timer per wait instead of one per
//! operation (`idle_active`), copy buffers grow under the process budget
//! (`tcp_copy_budget.rs`), and a running pump is an interactive presence.
//! The route actor sleeps while nothing asks for the route
//! (`route_monitor.rs`); a pump that moves data lets it observe once per
//! `OBSERVE_EVERY`, so route samples and the order of validated paths keep
//! following a tunnel as they did when every operation asked.
use crate::share::keepalive::interactive_presence::{self, InteractiveKind, InteractivePresence};
use crate::share::route_monitor;
use crate::share::route_protocol::{active_now, run_with_route};
use crate::share::route_state::RouteStatus;
use crate::share::tcp_registry::TcpCounters;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::{
    future::Future,
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

#[cfg(test)]
#[path = "tcp_pump_route_tests.rs"]
mod route_tests;
#[path = "tcp_copy_budget.rs"]
mod tcp_copy_budget;
#[cfg(test)]
#[path = "tcp_pump_tests.rs"]
mod tests;

use tcp_copy_budget::{CopyBudget, CopyBuffer};

const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const IDLE_MESSAGE: &str = "TCP connection idle for 120 active seconds";
/// Budget of one operation on the plain and legacy paths. The idle budget ends
/// a quiet pump far earlier, so this value never decides on its own.
const OPERATION_BUDGET: Duration = Duration::from_secs(86_400);
/// A copy direction that runs operations asks the route monitor at most this
/// often (`wait_application_route` wakes its actor, which then observes as
/// long as application bytes move). Half the two seconds between two route
/// samples (`route_monitor_samples.rs`).
const OBSERVE_EVERY: Duration = Duration::from_secs(1);

/// Process facts of one pump. Production values come from `production_env`;
/// tests pass their own register and budget.
#[derive(Clone, Copy)]
struct PumpEnv {
    presence: &'static InteractivePresence,
    budget: &'static CopyBudget,
    legacy: bool,
}

fn production_env() -> PumpEnv {
    PumpEnv {
        presence: interactive_presence::global(),
        budget: tcp_copy_budget::process_budget(),
        legacy: tcp_copy_budget::legacy_copy(),
    }
}

pub async fn pump(
    connection: &Connection,
    socket: &mut tokio::net::TcpStream,
    send: &mut SendStream,
    recv: &mut RecvStream,
    counters: Arc<TcpCounters>,
) -> io::Result<()> {
    pump_with(connection, socket, send, recv, counters, production_env()).await
}

async fn pump_with(
    connection: &Connection,
    socket: &mut tokio::net::TcpStream,
    send: &mut SendStream,
    recv: &mut RecvStream,
    counters: Arc<TcpCounters>,
    env: PumpEnv,
) -> io::Result<()> {
    counters.active.fetch_add(1, Ordering::Relaxed);
    struct Active(Arc<TcpCounters>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let _active = Active(counters.clone());
    let route = PumpRoute::open(connection, env.legacy)?;
    let (progress, idle_clock) = watch::channel(active_now());
    let (mut read, mut write) = socket.split();
    let transfer = both(
        &route,
        env,
        &progress,
        Lane {
            read: &mut read,
            write: send,
            count: &counters.read,
        },
        Lane {
            read: recv,
            write: &mut write,
            count: &counters.written,
        },
    );
    let idle = async move {
        if env.legacy {
            idle_legacy(idle_clock).await
        } else {
            idle_active(idle_clock, IDLE_TIMEOUT).await
        }
    };
    tokio::select! {
        result = transfer => result,
        error = idle => Err(error),
    }
}

/// One copy direction: its source, its destination and the byte counter it adds to.
struct Lane<'a, R, W> {
    read: &'a mut R,
    write: &'a mut W,
    count: &'a AtomicU64,
}

/// Both copy directions with one buffer each. The pump is present for interactive
/// purposes (E3) while this future lives; dropping it returns the buffers' budget.
async fn both<RA, WA, RB, WB>(
    route: &PumpRoute,
    env: PumpEnv,
    progress: &watch::Sender<tokio::time::Instant>,
    up: Lane<'_, RA, WA>,
    down: Lane<'_, RB, WB>,
) -> io::Result<()>
where
    RA: AsyncRead + Unpin,
    WA: AsyncWrite + Unpin,
    RB: AsyncRead + Unpin,
    WB: AsyncWrite + Unpin,
{
    let _presence = env.presence.enter(InteractiveKind::TcpPump);
    let mut up_route = route.clone();
    let mut down_route = route.clone();
    let mut up_buffer = CopyBuffer::new(env.budget, !env.legacy);
    let mut down_buffer = CopyBuffer::new(env.budget, !env.legacy);
    tokio::try_join!(
        copy(
            &mut up_route,
            up.read,
            up.write,
            up.count,
            progress,
            &mut up_buffer
        ),
        copy(
            &mut down_route,
            down.read,
            down.write,
            down.count,
            progress,
            &mut down_buffer
        ),
    )?;
    Ok(())
}

async fn copy<R, W>(
    route: &mut PumpRoute,
    read: &mut R,
    write: &mut W,
    count: &AtomicU64,
    progress: &watch::Sender<tokio::time::Instant>,
    buffer: &mut CopyBuffer,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let length = route.op(read.read(buffer.slot())).await?;
        if length == 0 {
            route.op(write.shutdown()).await?;
            progress.send_replace(active_now());
            return Ok(());
        }
        let full = length == buffer.size();
        route.op(write.write_all(buffer.filled(length))).await?;
        count.fetch_add(length as u64, Ordering::Relaxed);
        progress.send_replace(active_now());
        // While data flows the CPU stays awake in low power (a bounded,
        // deduplicated hold); a quiet tunnel holds nothing.
        crate::share::power::request_hold(crate::share::power::STREAM_HOLD_MS);
        // A read that filled the buffer means more data is waiting (E7 b).
        if full {
            buffer.grow_if_allowed();
        }
    }
}

/// How a pump waits for the application route before each operation.
#[derive(Clone)]
enum PumpRoute {
    /// No route monitor and no path policy: the plain per-operation budget of the raw Iroh contract.
    Plain,
    /// The copy mode before plan E7 (test switch): one `run_with_route` per operation.
    Legacy(Connection),
    /// The default: one route subscription for the whole pump.
    Shared {
        connection: Connection,
        status: watch::Receiver<RouteStatus>,
        /// When this direction last asked the route monitor (`OBSERVE_EVERY`).
        observed: tokio::time::Instant,
    },
}

impl PumpRoute {
    fn open(connection: &Connection, legacy: bool) -> io::Result<Self> {
        if legacy {
            return Ok(Self::Legacy(connection.clone()));
        }
        match route_monitor::subscribe(connection) {
            Some(status) => Ok(Self::Shared {
                connection: connection.clone(),
                status,
                observed: tokio::time::Instant::now(),
            }),
            None if !connection.application_path_policy().enabled => Ok(Self::Plain),
            None => Err(io::Error::other("Routemonitor fehlt")),
        }
    }

    async fn op<T>(&mut self, future: impl Future<Output = io::Result<T>>) -> io::Result<T> {
        match self {
            Self::Plain => tokio::time::timeout(OPERATION_BUDGET, future)
                .await
                .map_err(|_| share_deadline())?,
            Self::Legacy(connection) => {
                run_with_route(connection, OPERATION_BUDGET, None, future).await
            }
            Self::Shared {
                connection,
                status,
                observed,
            } => shared_op(connection, status, observed, future).await,
        }
    }
}

/// One operation on the shared subscription. A route pause waits with the
/// existing wait contract; the operation keeps its future across route changes
/// and is never restarted, and it is not polled while the route is paused.
/// Once per `OBSERVE_EVERY` the wait contract is asked although the route is
/// admitted: it returns at once and wakes the route actor.
async fn shared_op<T>(
    connection: &Connection,
    status: &mut watch::Receiver<RouteStatus>,
    observed: &mut tokio::time::Instant,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    tokio::pin!(future);
    loop {
        if !route_allows(connection, status) || observed.elapsed() >= OBSERVE_EVERY {
            route_monitor::wait_application_route(connection, None).await?;
            *observed = tokio::time::Instant::now();
        }
        tokio::select! {
            biased;
            result = &mut future => return result,
            result = status.changed() => if result.is_err() {
                return Err(io::Error::other("Routemonitor beendet"));
            },
            _ = connection.closed() => {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Share-Verbindung geschlossen",
                ));
            }
        }
    }
}

fn route_allows(connection: &Connection, status: &watch::Receiver<RouteStatus>) -> bool {
    let application_allowed = status.borrow().application_allowed;
    application_allowed && connection.application_data_allowed()
}

fn share_deadline() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "Share-Anwendungsfrist abgelaufen")
}

/// Ends the pump after `limit` of active time without a completed operation.
/// Progress does not wake it: it sleeps for the remaining budget and checks
/// again, so route pauses add no active time and no operation adds a timer.
/// A check never follows the last one sooner than one hundredth of the limit:
/// during a route pause the remaining budget stands still, and a pause that
/// begins just before the budget ends must not turn the wait into a spin.
async fn idle_active(
    progress: watch::Receiver<tokio::time::Instant>,
    limit: Duration,
) -> io::Error {
    loop {
        let last = *progress.borrow();
        let quiet = active_now().saturating_duration_since(last);
        match limit.checked_sub(quiet) {
            Some(rest) if !rest.is_zero() => tokio::time::sleep(rest.max(limit / 100)).await,
            _ => return io::Error::new(io::ErrorKind::TimedOut, IDLE_MESSAGE),
        }
    }
}

/// The copy mode before plan E7: every operation wakes the idle budget.
async fn idle_legacy(mut progress: watch::Receiver<tokio::time::Instant>) -> io::Error {
    loop {
        let deadline = *progress.borrow_and_update() + IDLE_TIMEOUT;
        tokio::select! {
            result = progress.changed() => if result.is_err() {
                return io::Error::other("TCP pump stopped");
            },
            () = crate::share::route_protocol::sleep_until_active(deadline) => {
                return io::Error::new(io::ErrorKind::TimedOut, IDLE_MESSAGE);
            }
        }
    }
}
