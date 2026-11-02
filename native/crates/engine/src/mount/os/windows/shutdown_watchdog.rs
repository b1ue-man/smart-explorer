use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SLOW_SHUTDOWN_PHASE: Duration = Duration::from_millis(500);

pub(super) struct ShutdownWatchdog {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

struct State {
    stopped: bool,
    phase: &'static str,
    changed_at: Instant,
    reported: bool,
}

impl ShutdownWatchdog {
    pub(super) fn start(phase: &'static str) -> Option<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                stopped: false,
                phase,
                changed_at: Instant::now(),
                reported: false,
            }),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("mount-shutdown-watchdog".into())
            .spawn(move || run(worker))
            .ok()?;
        Some(Self {
            shared,
            thread: Some(thread),
        })
    }

    pub(super) fn set_phase(&self, phase: &'static str) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.phase = phase;
            state.changed_at = Instant::now();
            state.reported = false;
            self.shared.wake.notify_all();
        }
    }

    pub(super) fn finish(mut self) -> bool {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> bool {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.stopped = true;
        self.shared.wake.notify_all();
        drop(state);
        if let Some(thread) = self.thread.take() {
            thread.join().is_ok()
        } else {
            true
        }
    }
}

impl Drop for ShutdownWatchdog {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

fn run(shared: Arc<Shared>) {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    loop {
        if state.stopped {
            return;
        }
        if state.reported {
            state = match shared.wake.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            continue;
        }
        let deadline = state.changed_at + SLOW_SHUTDOWN_PHASE;
        let now = Instant::now();
        if now >= deadline {
            let phase = state.phase;
            let elapsed = now.saturating_duration_since(state.changed_at);
            state.reported = true;
            drop(state);
            let _ = writeln!(
                std::io::stderr().lock(),
                "mount shutdown still blocked: phase={}, elapsed_ms={}",
                phase,
                elapsed.as_millis()
            );
            state = match shared.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            continue;
        }
        state = match shared.wake.wait_timeout(state, deadline - now) {
            Ok((state, _)) => state,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
}
