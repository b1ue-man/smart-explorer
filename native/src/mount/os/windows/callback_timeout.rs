use std::collections::HashMap;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::{DokanFileInfo, DokanyRuntime};

const RESET_INTERVAL: Duration = Duration::from_secs(30);
const MAX_SUPERVISED_CALLBACKS: usize = 4_096;
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
}

struct Request {
    file_info: usize,
    state: Mutex<RequestState>,
    wake: Condvar,
}

struct RequestState {
    next_reset: Instant,
    failed: bool,
    in_flight: bool,
    reported: bool,
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
                if catch_unwind(AssertUnwindSafe(|| run(runtime, Arc::clone(&worker)))).is_err() {
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
        if state.requests.len() >= MAX_SUPERVISED_CALLBACKS {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "too many supervised filesystem callbacks",
            ));
        }
        let id = allocate_id(&mut state)?;
        state.requests.insert(
            id,
            Arc::new(Request {
                file_info: file_info as usize,
                state: Mutex::new(RequestState {
                    next_reset: Instant::now() + RESET_INTERVAL,
                    failed: false,
                    in_flight: false,
                    reported: false,
                }),
                wake: Condvar::new(),
            }),
        );
        self.shared.wake.notify_all();
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
        self.shared.wake.notify_all();
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
        request_state.next_reset = Instant::now() + RESET_INTERVAL;
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

fn allocate_id(state: &mut State) -> io::Result<u64> {
    for _ in 0..u16::MAX {
        state.next_id = state.next_id.wrapping_add(1).max(1);
        if !state.requests.contains_key(&state.next_id) {
            return Ok(state.next_id);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::OutOfMemory,
        "too many supervised filesystem callbacks",
    ))
}

fn run(runtime: DokanyRuntime, shared: Arc<Shared>) {
    let mut state = match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    loop {
        if state.stopped {
            return;
        }
        let Some(next_reset) = state
            .requests
            .values()
            .map(|request| lock_request(request).next_reset)
            .min()
        else {
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
        let now = Instant::now();
        let due = state
            .requests
            .values()
            .filter_map(|request| {
                let request_state = lock_request(request);
                (request_state.next_reset <= now && !request_state.in_flight)
                    .then_some((request_state.next_reset, Arc::clone(request)))
            })
            .min_by_key(|(deadline, _)| *deadline)
            .map(|(_, request)| request);
        let Some(request) = due else {
            continue;
        };
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
        let claim = ResetClaim::new(request);
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
