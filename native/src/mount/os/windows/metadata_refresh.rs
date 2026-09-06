use crate::mount::MountEngine;
use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const REFRESH_INTERVAL: Duration = Duration::from_secs(20);

pub(super) struct MetadataRefreshWorker {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl MetadataRefreshWorker {
    pub(super) fn start(engine: Arc<MountEngine>) -> io::Result<Self> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("mount-metadata-refresh".into())
            .spawn(move || run(engine, worker_stop))?;
        Ok(Self {
            stop,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(super) fn stop(&self) {
        self.request_stop();
        self.join();
    }

    pub(super) fn request_stop(&self) {
        let (state, wake) = &*self.stop;
        if let Ok(mut stopped) = state.lock() {
            *stopped = true;
            wake.notify_all();
        }
    }

    pub(super) fn join(&self) {
        let thread = self.thread.lock().ok().and_then(|mut thread| thread.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for MetadataRefreshWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(engine: Arc<MountEngine>, stop: Arc<(Mutex<bool>, Condvar)>) {
    let mut next_refresh = Instant::now() + REFRESH_INTERVAL;
    loop {
        if is_stopped(&stop) {
            return;
        }
        let _ = engine.maintain_cache();
        if Instant::now() >= next_refresh {
            let _ = engine.refresh_metadata_while(|| is_stopped(&stop));
            next_refresh = Instant::now() + REFRESH_INTERVAL;
        }
        let progressed = engine
            .preload_metadata_batch_while(|| is_stopped(&stop))
            .unwrap_or(0);
        // Productive bounded expansion can immediately refill. The configured
        // depth, retention budget, backend admission and stop checks still apply.
        if progressed > 0 { continue; }
        let delay = next_refresh.saturating_duration_since(Instant::now());
        if wait_for_stop(&stop, delay) {
            return;
        }
    }
}

fn is_stopped(stop: &(Mutex<bool>, Condvar)) -> bool {
    stop.0.lock().map_or(true, |stopped| *stopped)
}

fn wait_for_stop(stop: &(Mutex<bool>, Condvar), delay: Duration) -> bool {
    let Ok(stopped) = stop.0.lock() else {
        return true;
    };
    if *stopped {
        return true;
    }
    stop.1
        .wait_timeout(stopped, delay)
        .map_or(true, |(stopped, _)| *stopped)
}
