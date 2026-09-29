use std::io;
use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::core::eio;

// Control pool: short metadata operations (listing, stat, renames, removals)
// and the tree walks. Transfers no longer draw from it; they hold their own
// admission slot (`spawn_holding`), so long reads and writes never delay
// browsing. A Share mount keeps at most eight requests live (daemon request
// workers), so 32 serve four busy mounts or browsers at once and leave most of
// Tokio's 512 blocking threads to transfers. Acquisition stays asynchronous
// and therefore never occupies an Iroh worker.
const MAX_BLOCKING_OPERATIONS: usize = 32;

fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_BLOCKING_OPERATIONS)))
        .clone()
}

pub(super) struct BlockingTask<T> {
    operation: &'static str,
    handle: JoinHandle<io::Result<T>>,
}

impl<T: Send + 'static> BlockingTask<T> {
    pub(super) async fn join(self) -> io::Result<T> {
        self.handle.await.map_err(|error| {
            eio(format!(
                "{} blocking worker failed: {error}",
                self.operation
            ))
        })?
    }
}

pub(super) async fn spawn<T, F>(operation: &'static str, work: F) -> io::Result<BlockingTask<T>>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    let permit = slots()
        .acquire_owned()
        .await
        .map_err(|_| eio("Share blocking worker is closed"))?;
    let handle = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });
    Ok(BlockingTask { operation, handle })
}

pub(super) async fn run<T, F>(operation: &'static str, work: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    spawn(operation, work).await?.join().await
}

/// Runs a transfer on the blocking pool while `slot` (its admission) stays
/// held; it bypasses the control pool, which the slot already bounds.
pub(super) fn spawn_holding<T, G, F>(operation: &'static str, slot: G, work: F) -> BlockingTask<T>
where
    T: Send + 'static,
    G: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    let handle = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        work()
    });
    BlockingTask { operation, handle }
}

pub(super) async fn run_holding<T, G, F>(operation: &'static str, slot: G, work: F) -> io::Result<T>
where
    T: Send + 'static,
    G: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    spawn_holding(operation, slot, work).join().await
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read, Write};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};

    use super::run;

    struct BlockingBackend {
        started: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl Backend for BlockingBackend {
        fn scheme(&self) -> Scheme {
            Scheme::Peer
        }

        fn root_display(&self) -> String {
            "/".into()
        }

        fn list_dir(&self, _path: &str) -> VfsResult<Vec<VfsMeta>> {
            Ok(Vec::new())
        }

        fn stat(&self, _path: &str) -> VfsResult<VfsMeta> {
            let (lock, changed) = &*self.release;
            if let Some(started) = self.started.lock().unwrap().take() {
                let _ = started.send(());
            }
            let mut released = lock.lock().unwrap();
            while !*released {
                released = changed.wait(released).unwrap();
            }
            Ok(VfsMeta::default())
        }

        fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        fn rename(&self, _src: &str, _dst: &str) -> VfsResult<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        fn remove_file(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        fn remove_dir(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }

        fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "unused"))
        }
    }

    #[test]
    fn blocked_backend_does_not_starve_timer_or_independent_work() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let release = Arc::new((Mutex::new(false), Condvar::new()));
            let backend = BlockingBackend {
                started: Mutex::new(Some(started_tx)),
                release: release.clone(),
            };
            let blocked = tokio::spawn(run("blocked backend stat", move || backend.stat("/")));
            started_rx.await.unwrap();

            let timer_progressed = tokio::time::timeout(Duration::from_secs(1), async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                true
            })
            .await
            .unwrap_or(false);
            let independent_progressed = tokio::time::timeout(
                Duration::from_secs(1),
                run("independent share request", || Ok(7usize)),
            )
            .await;

            let (lock, changed) = &*release;
            *lock.lock().unwrap() = true;
            changed.notify_all();
            let blocked_result = tokio::time::timeout(Duration::from_secs(1), blocked).await;

            assert!(timer_progressed);
            assert_eq!(independent_progressed.unwrap().unwrap(), 7);
            assert!(blocked_result.unwrap().unwrap().is_ok());
        });
    }
}
