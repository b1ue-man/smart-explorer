//! Shared deadline contract: one future, paused active budget, unchanged total deadline.
use iroh::endpoint::Connection;
use std::{
    future::Future,
    io,
    time::{Duration, Instant},
};

tokio::task_local! { static CONNECTION: Connection; }

pub(crate) async fn scope<T>(connection: Connection, future: impl Future<Output = T>) -> T {
    CONNECTION.scope(connection, future).await
}

pub(crate) fn current_connection() -> Option<Connection> {
    CONNECTION.try_with(Clone::clone).ok()
}

/// A task-local monotone active clock. OS grant expiry and caller total deadlines
/// continue to use the ordinary wall/monotone clock, never this paused clock.
pub(crate) fn active_now() -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    let paused = current_connection().map_or(Duration::ZERO, |connection| {
        super::route_monitor::paused_duration(&connection)
    });
    now.checked_sub(paused).unwrap_or(now)
}

pub(crate) async fn timeout_until<T>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = T>,
) -> io::Result<T> {
    timeout_for(deadline.saturating_duration_since(active_now()), future).await
}

pub(crate) async fn timeout_for<T>(
    budget: Duration,
    future: impl Future<Output = T>,
) -> io::Result<T> {
    if let Some(connection) = current_connection() {
        run_with_route(&connection, budget, None, async { Ok(future.await) }).await
    } else {
        tokio::time::timeout(budget, future).await.map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "Share-Anwendungsfrist abgelaufen")
        })
    }
}

pub(crate) async fn sleep_until_active(deadline: tokio::time::Instant) {
    let _ = timeout_until(deadline, std::future::pending::<()>()).await;
}

pub(crate) async fn active_deadline_error(deadline: tokio::time::Instant) -> io::Error {
    match timeout_until(deadline, std::future::pending::<()>()).await {
        Err(error) => error,
        Ok(()) => io::Error::new(io::ErrorKind::TimedOut, "Share-Anwendungsfrist abgelaufen"),
    }
}

pub(crate) async fn run_until_active<T>(
    deadline: tokio::time::Instant,
    message: &'static str,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    timeout_until(deadline, future).await.map_err(|error| {
        if error.kind() == io::ErrorKind::TimedOut {
            io::Error::new(io::ErrorKind::TimedOut, message)
        } else {
            error
        }
    })?
}

pub(crate) async fn close_if_abandoned_during_pause(
    connection: &Connection,
    abandoned: impl Future<Output = ()>,
) {
    abandoned.await;
    let Some(mut status) = super::route_monitor::subscribe(connection) else {
        let _ = connection.closed().await;
        return;
    };
    loop {
        if !connection.application_data_allowed() {
            connection.close(
                iroh::endpoint::VarInt::from_u32(0x4558),
                b"Exec owner closed during route pause",
            );
            return;
        }
        if status.changed().await.is_err() {
            return;
        }
    }
}

/// The future is constructed once and is never restarted after a route change.
/// Cancellation remains an outer select/drop, including while the route is paused.
pub(crate) async fn run_with_route<T>(
    connection: &Connection,
    active_budget: Duration,
    total_deadline: Option<Instant>,
    future: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    // Generic raw-Iroh callers keep their existing non-policy contract. Every
    // ShareIrohNode endpoint opts in before connection construction.
    if super::route_monitor::subscribe(connection).is_none()
        && !connection.application_path_policy().enabled
    {
        let budget = total_deadline.map_or(active_budget, |deadline| {
            active_budget.min(deadline.saturating_duration_since(Instant::now()))
        });
        return tokio::time::timeout(budget, future).await.map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "Share-Anwendungsfrist abgelaufen")
        })?;
    }
    let mut status = super::route_monitor::subscribe(connection)
        .ok_or_else(|| io::Error::other("Routemonitor fehlt"))?;
    tokio::pin!(future);
    let mut remaining = active_budget;
    loop {
        super::route_monitor::wait_application_route(connection, total_deadline).await?;
        let started = tokio::time::Instant::now();
        let total = async {
            if let Some(deadline) = total_deadline {
                tokio::time::sleep_until(deadline.into()).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            _ = total => return Err(io::Error::new(io::ErrorKind::TimedOut, "Verbindungsaufbau/Pfadwechsel: Gesamtfrist abgelaufen")),
            result = status.changed() => if result.is_err() { return Err(io::Error::other("Routemonitor beendet")); },
            result = &mut future => return result,
            _ = tokio::time::sleep(remaining) => return Err(io::Error::new(io::ErrorKind::TimedOut, "Share-Anwendungsfrist abgelaufen")),
            _ = connection.closed() => return Err(io::Error::new(io::ErrorKind::NotConnected, "Share-Verbindung geschlossen")),
        }
        remaining = remaining.saturating_sub(started.elapsed());
    }
}
