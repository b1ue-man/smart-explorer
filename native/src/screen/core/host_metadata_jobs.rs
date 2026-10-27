//! Finite platform queries never run on the async executor or under the auth lock.
use super::host_backend::ScreenHostBackend;
use super::host_server::HostContext;
use crate::share::{access_grant::Capability, relation_proof::ProvenPrincipal};
use std::io;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, Weak,
};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

// The metadata channel already treats 15 seconds without progress as a lost viewer.
const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
type Lane = (Weak<dyn ScreenHostBackend>, Weak<Semaphore>);

fn lane(backend: &Arc<dyn ScreenHostBackend>) -> io::Result<Arc<Semaphore>> {
    static LANES: OnceLock<Mutex<Vec<Lane>>> = OnceLock::new();
    let mut lanes = LANES
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| io::Error::other("screen metadata lanes unavailable"))?;
    lanes.retain(|(backend, lane)| backend.strong_count() > 0 && lane.strong_count() > 0);
    for (known, lane) in lanes.iter() {
        if known
            .upgrade()
            .is_some_and(|known| Arc::ptr_eq(&known, backend))
        {
            if let Some(lane) = lane.upgrade() {
                return Ok(lane);
            }
        }
    }
    // Preserve the previous auth-mutex serialization for each actual backend.
    // A cancelled but running query keeps this permit until it really finishes.
    let lane = Arc::new(Semaphore::new(1));
    lanes.push((Arc::downgrade(backend), Arc::downgrade(&lane)));
    Ok(lane)
}

pub(crate) fn check(
    context: &HostContext,
    principal: &ProvenPrincipal,
    needed: &[Capability],
) -> io::Result<()> {
    if context
        .authority
        .admit(principal, needed, crate::share::core_now_secs(), &mut || {})
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "screen metadata permission is no longer valid",
        ))
    }
}

fn active_check(active: &AtomicBool) -> io::Result<()> {
    if active.load(Ordering::Acquire) {
        Ok(())
    } else {
        Err(io::Error::from(io::ErrorKind::Interrupted))
    }
}

struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Owns queued and running work. Drop cancels queued work and discards late results.
pub(crate) struct Job<T> {
    task: JoinHandle<io::Result<T>>,
    active: Arc<AtomicBool>,
    deadline: tokio::time::Instant,
}

impl<T> Job<T> {
    pub(crate) async fn result(&mut self) -> io::Result<T> {
        tokio::time::timeout_at(self.deadline, &mut self.task)
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "screen metadata query timed out")
            })?
            .map_err(io::Error::other)?
    }
}
impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        self.task.abort();
    }
}

/// A viewer's window, icon or family query: one at a time per backend, as
/// under the authorization lock before.
pub(crate) fn spawn<T, F>(
    context: &HostContext,
    principal: &ProvenPrincipal,
    needed: &'static [Capability],
    work: F,
) -> Job<T>
where
    T: Send + 'static,
    F: FnOnce(&HostContext, &ProvenPrincipal) -> io::Result<T> + Send + 'static,
{
    start(context, principal, needed, true, work)
}

/// A session start's capability or memory query. It never waited behind a
/// window listing, so a slow listing cannot delay or refuse a start.
pub(crate) fn spawn_start<T, F>(
    context: &HostContext,
    principal: &ProvenPrincipal,
    needed: &'static [Capability],
    work: F,
) -> Job<T>
where
    T: Send + 'static,
    F: FnOnce(&HostContext, &ProvenPrincipal) -> io::Result<T> + Send + 'static,
{
    start(context, principal, needed, false, work)
}

fn start<T, F>(
    context: &HostContext,
    principal: &ProvenPrincipal,
    needed: &'static [Capability],
    serialized: bool,
    work: F,
) -> Job<T>
where
    T: Send + 'static,
    F: FnOnce(&HostContext, &ProvenPrincipal) -> io::Result<T> + Send + 'static,
{
    let context = context.clone();
    let principal = principal.clone();
    let active = Arc::new(AtomicBool::new(true));
    let running = active.clone();
    let task = tokio::spawn(async move {
        active_check(&running)?;
        check(&context, &principal, needed)?;
        let permit = if serialized {
            let lane = lane(&context.backend)?;
            Some(lane.acquire_owned().await.map_err(io::Error::other)?)
        } else {
            None
        };
        let mut worker = AbortOnDrop(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            active_check(&running)?;
            check(&context, &principal, needed)?;
            let result = work(&context, &principal);
            active_check(&running)?;
            check(&context, &principal, needed)?;
            result
        }));
        (&mut worker.0).await.map_err(io::Error::other)?
    });
    Job {
        task,
        active,
        deadline: tokio::time::Instant::now() + QUERY_TIMEOUT,
    }
}
