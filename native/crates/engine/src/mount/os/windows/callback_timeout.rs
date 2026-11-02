use std::collections::HashMap;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{DokanFileInfo, DokanyRuntime};

mod schedule;
use schedule::ResetSchedule;

#[cfg(test)]
mod bulk_timeout_task_tests;

const RESET_INTERVAL: Duration = Duration::from_secs(30);
const MAX_ACTIVE_REPORTS: usize = 32;

pub(super) struct CallbackTimeoutSupervisor {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

#[derive(Default)]
struct State {
    stopped: bool,
    failed: bool,
    next_id: u64,
    emitted_reports: usize,
    requests: HashMap<u64, Arc<Request>>,
    schedule: ResetSchedule,
}

struct Request {
    file_info: usize,
    state: Mutex<RequestState>,
    wake: Condvar,
}

struct RequestState {
    failed: bool,
    in_flight: bool,
    reported: bool,
}

impl State {
    fn rearm_registered(&mut self, id: u64, request: &Arc<Request>, now: Instant) -> io::Result<()> {
        if !self.stopped
            && self.requests.get(&id)
                .is_some_and(|registered| Arc::ptr_eq(registered, request))
        {
            self.schedule.arm(id, now)?;
        }
        Ok(())
    }
}

impl CallbackTimeoutSupervisor {
    pub(super) fn start(runtime: DokanyRuntime) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("mount-timeout-supervisor".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| run(runtime, Arc::clone(&worker))));
                if !matches!(result, Ok(Ok(()))) {
                    fail_all(&worker);
                }
            })?;
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(super) fn register(
        &self,
        file_info: *mut DokanFileInfo,
    ) -> io::Result<CallbackTimeoutLease<'_>> {
        if file_info.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing Dokany request for timeout supervision",
            ));
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| io::Error::other("callback timeout supervisor is unavailable"))?;
        if state.stopped {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "callback timeout supervisor is stopping",
            ));
        }
        let id = allocate_id(&mut state)?;
        state.requests.try_reserve(1).map_err(|_| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "callback timeout registration allocation failed",
            )
        })?;
        state.requests.insert(
            id,
            Arc::new(Request {
                file_info: file_info as usize,
                state: Mutex::new(RequestState {
                    failed: false,
                    in_flight: false,
                    reported: false,
                }),
                wake: Condvar::new(),
            }),
        );
        match state.schedule.arm(id, Instant::now()) {
            Ok(true) => self.shared.wake.notify_one(),
            Ok(false) => {}
            Err(error) => {
                state.requests.remove(&id);
                shrink_requests(&mut state);
                return Err(error);
            }
        }
        Ok(CallbackTimeoutLease {
            supervisor: self,
            id: Some(id),
        })
    }

    fn finish(&self, id: u64) -> bool {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let request = state.requests.remove(&id);
        shrink_requests(&mut state);
        if state.schedule.remove(id) {
            self.shared.wake.notify_one();
        }
        drop(state);
        let Some(request) = request else {
            return false;
        };
        let mut request_state = lock_request(&request);
        while request_state.in_flight {
            request_state = match request.wake.wait(request_state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
        !request_state.failed
    }

    pub(super) fn failed(&self) -> bool {
        match self.shared.state.lock() {
            Ok(state) => state.failed,
            Err(_) => true,
        }
    }

    fn stop_and_join(&self) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.stopped = true;
        self.shared.wake.notify_all();
        drop(state);
        let thread = self.thread.lock().ok().and_then(|mut thread| thread.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for CallbackTimeoutSupervisor {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

pub(super) struct CallbackTimeoutLease<'a> {
    supervisor: &'a CallbackTimeoutSupervisor,
    id: Option<u64>,
}

impl CallbackTimeoutLease<'_> {
    pub(super) fn finish(mut self) -> bool {
        self.id.take().is_some_and(|id| self.supervisor.finish(id))
    }
}

impl Drop for CallbackTimeoutLease<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.supervisor.finish(id);
        }
    }
}

struct ResetClaim {
    request: Arc<Request>,
    completed: bool,
}

impl ResetClaim {
    fn new(request: Arc<Request>) -> Self {
        Self {
            request,
            completed: false,
        }
    }

    fn complete(mut self, reset: bool) {
        let mut request_state = lock_request(&self.request);
        request_state.failed |= !reset;
        request_state.in_flight = false;
        self.request.wake.notify_all();
        self.completed = true;
    }
}

impl Drop for ResetClaim {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut request_state = lock_request(&self.request);
        request_state.failed = true;
        request_state.in_flight = false;
        self.request.wake.notify_all();
    }
}

fn shrink_requests(state: &mut State) {
    let capacity = state.requests.capacity();
    let live = state.requests.len();
    // Geometric hysteresis bounds whole-burst rebuild work. Moving Arc values
    // does not move requests, and finish still owns its removed Arc until the
    // claimed reset completes. No request mutex is acquired for maintenance.
    if capacity > 32 && capacity / 4 > live {
        state.requests.shrink_to(live.saturating_mul(2));
    }
}

fn allocate_id(state: &mut State) -> io::Result<u64> {
    state.next_id = state.next_id
        .checked_add(1)
        .ok_or_else(|| io::Error::other("callback timeout request ID space exhausted"))?;
    Ok(state.next_id)
}

fn run(runtime: DokanyRuntime, shared: Arc<Shared>) -> io::Result<()> {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    loop {
        if state.stopped {
            return Ok(());
        }
        let Some((id, next_reset)) = state.schedule.front() else {
            state = match shared.wake.wait(state) {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            continue;
        };
        let now = Instant::now();
        if now < next_reset {
            state = match shared.wake.wait_timeout(state, next_reset - now) {
                Ok((state, _)) => state,
                Err(poisoned) => poisoned.into_inner().0,
            };
            continue;
        }
        let request = state.requests.get(&id).cloned()
            .ok_or_else(|| io::Error::other("scheduled callback is no longer registered"))?;
        state.schedule.remove(id);
        // Claim while holding shared -> request locks, so finish cannot remove
        // the registration and release file_info before in_flight is visible.
        let claim = ResetClaim::new(Arc::clone(&request));
        let should_report = {
            let mut request_state = lock_request(&request);
            request_state.in_flight = true;
            if !request_state.reported && state.emitted_reports < MAX_ACTIVE_REPORTS {
                request_state.reported = true;
                state.emitted_reports += 1;
                true
            } else {
                false
            }
        };
        let file_info = claim.request.file_info;
        drop(state);
        if should_report {
            let _ = writeln!(
                std::io::stderr().lock(),
                "mount callback still running after {} ms",
                RESET_INTERVAL.as_millis()
            );
        }
        let reset = unsafe {
            runtime.reset_timeout(
                super::callback_status::CALLBACK_TIMEOUT_MS,
                file_info as *mut DokanFileInfo,
            )
        };
        claim.complete(reset);
        state = match shared.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        // Completion released the request mutex before taking shared again.
        // A concurrent finish may have removed this ID while waiting for the
        // FFI reset. Never rearm a removed request or retain a stale queue ticket.
        state.rearm_registered(id, &request, Instant::now())?;
    }
}

fn fail_all(shared: &Shared) {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.stopped = true;
    state.failed = true;
    let requests = state.requests.values().cloned().collect::<Vec<_>>();
    shared.wake.notify_all();
    drop(state);
    for request in requests {
        let mut request_state = lock_request(&request);
        request_state.failed = true;
        request_state.in_flight = false;
        request.wake.notify_all();
    }
}

fn lock_request(request: &Request) -> MutexGuard<'_, RequestState> {
    match request.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}
