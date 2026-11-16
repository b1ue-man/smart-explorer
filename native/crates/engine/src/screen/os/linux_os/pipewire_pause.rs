//! Pause shared by portal variants; retains the granted stream and rejects expired sources.
use super::dmabuf::{BufferReturn, DmaBufSession, Fallback, GpuSlot, GpuTaken, HeldBuffer};
use super::*;

#[derive(Default)]
struct LatestInner {
    seq: u64,
    taken: u64,
    image: Option<CapturedImage>,
    failure: Option<String>,
    stream_state: c_int,
}

/// The newest picture of the stream and its failure, shared between the
/// loop's thread and the capture.
pub struct LatestFrame {
    inner: Mutex<LatestInner>,
    pub(super) trace: Trace,
    control: Mutex<Option<Control>>,
    active: std::sync::atomic::AtomicBool,
    admitted: std::sync::atomic::AtomicBool,
    imports_blocked: std::sync::atomic::AtomicBool,
    state_changed: std::sync::Condvar,
    failed: tokio::sync::Notify,
    native_stream: AtomicPtr<c_void>,
    /// DMA-BUF negotiation and the newest held DMA-BUF picture (BK8b).
    pub(super) dmabuf: DmaBufSession,
    pub(super) gpu: GpuSlot,
    /// Monotonic token per added buffer, preventing ABA reuse across renegotiations.
    gpu_seq: std::sync::atomic::AtomicU64,
    /// Buffers the stream added and has not removed: only those go back.
    gpu_live: Mutex<Vec<(usize, u64)>>,
    /// The library for queueing on the loop thread, where `control` must
    /// not be locked (a worker may hold it while waiting for the loop).
    gpu_api: std::sync::OnceLock<Arc<PipewireApi>>,
}

impl LatestFrame {
    /// Whether the stream negotiated DMA-BUF pictures (BK8b).
    pub(in crate::screen::os) fn dmabuf_negotiated(&self) -> bool {
        self.dmabuf.dmabuf().is_some()
    }

    fn lock(&self) -> MutexGuard<'_, LatestInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn publish(&self, image: CapturedImage) {
        if !self.is_capture_active() {
            return;
        }
        let mut inner = self.lock();
        inner.seq += 1;
        inner.image = Some(image);
    }

    pub fn check_failure(&self) -> io::Result<()> {
        match &self.lock().failure {
            Some(reason) => Err(io::Error::other(reason.clone())),
            None => Ok(()),
        }
    }
    /// What the stream did so far (the failure text of a silent stream).
    #[cfg(any(test, feature = "test-support"))]
    pub fn describe(&self) -> String {
        let pictures = self.lock().seq;
        format!("{pictures} pictures; {}", self.trace.describe())
    }

    /// The stream runs, but every buffer it sent so far was an empty or
    /// corrupted chunk: it only reports cursor moves, no screen change (the
    /// source sends a picture only when the screen changes).
    #[cfg(any(test, feature = "test-support"))]
    pub fn only_empty_chunks(&self) -> bool {
        self.lock().seq == 0 && self.trace.only_empty_chunks()
    }

    pub fn fail(&self, reason: String) {
        self.admitted.store(false, Ordering::Release);
        self.lock().failure.get_or_insert(reason);
        self.failed.notify_one();
        self.state_changed.notify_all();
    }

    pub async fn wait_failure(&self) {
        loop {
            let notified = self.failed.notified();
            if self.lock().failure.is_some() {
                return;
            }
            notified.await;
        }
    }
    /// A picture newer than the last one taken; else `Unchanged`, but with
    /// `keyframe` the last picture again (a key frame always has one).
    pub fn take(&self, keyframe: bool) -> Taken {
        let mut inner = self.lock();
        if let Some(reason) = &inner.failure {
            return Taken::Failed(reason.clone());
        }
        if !self.is_capture_active() {
            return Taken::Unchanged;
        }
        let fresh = inner.seq != inner.taken;
        inner.taken = inner.seq;
        match &inner.image {
            Some(image) if fresh || keyframe => Taken::Frame(image.clone()),
            _ => Taken::Unchanged,
        }
    }
}

impl Default for LatestFrame {
    fn default() -> Self {
        Self {
            inner: Mutex::default(),
            trace: Trace::default(),
            control: Mutex::new(None),
            active: std::sync::atomic::AtomicBool::new(true),
            admitted: std::sync::atomic::AtomicBool::new(true),
            imports_blocked: std::sync::atomic::AtomicBool::new(false),
            state_changed: std::sync::Condvar::new(),
            failed: tokio::sync::Notify::new(),
            native_stream: AtomicPtr::new(null_mut()),
            dmabuf: DmaBufSession::default(),
            gpu: GpuSlot::default(),
            gpu_seq: std::sync::atomic::AtomicU64::new(0),
            gpu_live: Mutex::new(Vec::new()),
            gpu_api: std::sync::OnceLock::new(),
        }
    }
}
struct Control {
    api: Arc<PipewireApi>,
    thread_loop: *mut c_void,
}
// SAFETY: access is serialized by control and PipeWire's thread-loop lock; detach precedes destruction.
unsafe impl Send for Control {}
impl LatestFrame {
    pub(super) fn is_capture_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
            && self.admitted.load(Ordering::Acquire)
            && !self.import_blocked()
    }
    pub(super) fn on_state_changed(&self, old: c_int, new: c_int) {
        self.lock().stream_state = new;
        if new == 0 && old >= 2 {
            self.fail("pipewire_stream_disconnected".into());
        }
        self.state_changed.notify_all();
    }
    pub(super) fn block_import(&self, blocked: bool) {
        self.imports_blocked.store(blocked, Ordering::Release);
    }
    pub(super) fn import_blocked(&self) -> bool {
        self.imports_blocked.load(Ordering::Acquire)
    }
    pub(super) fn close_copy(&self) {
        self.active.store(false, Ordering::Release);
        self.admitted.store(false, Ordering::Release);
        self.lock().image = None;
        // No lock is held here (PwStream::drop): the drop gives it back.
        drop(self.gpu.clear());
    }
    pub fn set_initial_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }
    pub(super) fn suspend_before_connect(&self) {
        self.admitted.store(false, Ordering::Release);
    }
    pub(super) fn pending_format(&self) {
        self.admitted.store(false, Ordering::Release);
        {
            let mut inner = self.lock();
            inner.image = None;
            inner.taken = inner.seq;
        }
        // Called on the loop thread with its lock (param_changed).
        self.clear_gpu_locked();
    }
    // The caller holds the loop lock and already owns the exact stream.
    // No control mutex is taken: set_capture_active takes that before loop.
    pub(super) fn admitted_locked(&self) {
        self.admitted.store(true, Ordering::Release);
    }
    pub(super) fn desired_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }
    pub(super) fn attach(
        &self,
        api: Arc<PipewireApi>,
        thread_loop: *mut c_void,
        stream: *mut c_void,
    ) {
        self.native_stream.store(stream, Ordering::Release);
        let _ = self.gpu_api.set(api.clone());
        *self.control.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(Control { api, thread_loop });
    }
    pub(super) fn stream_destroyed(&self) {
        self.native_stream.store(null_mut(), Ordering::Release);
    }
    pub(super) fn detach(&self) {
        self.stream_destroyed();
        self.control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.active.store(false, Ordering::Release);
        self.fail("pipewire_stream_closed".into());
    }
    pub fn set_capture_active(&self, active: bool) -> io::Result<()> {
        let control = self
            .control
            .lock()
            .map_err(|_| io::Error::other("pipewire pause lock unavailable"))?;
        let failure = self.lock().failure.clone();
        if active {
            if let Some(failure) = failure {
                return Err(io::Error::other(failure));
            }
        }
        if let Some(control) = control.as_ref() {
            // SAFETY: owner cannot destroy these objects while control is locked.
            let result = unsafe {
                (control.api.thread_loop_lock)(control.thread_loop);
                self.active.store(active, Ordering::Release);
                {
                    let mut inner = self.lock();
                    inner.image = None;
                    inner.taken = inner.seq;
                }
                self.clear_gpu_locked();
                let stream = self.native_stream.load(Ordering::Acquire);
                let result = if stream.is_null() {
                    -libc::ENOTCONN
                } else {
                    (control.api.stream_set_active)(stream, active)
                };
                if result < 0 {
                    self.active.store(false, Ordering::Release);
                }
                (control.api.thread_loop_unlock)(control.thread_loop);
                result
            };
            if result < 0 {
                return Err(io::Error::other(format!(
                    "pw_stream_set_active failed: {result}"
                )));
            }
        } else {
            self.active.store(active, Ordering::Release);
            let mut inner = self.lock();
            inner.image = None;
            inner.taken = inner.seq;
        }
        let mut inner = self.lock();
        if !active {
            let (state, timeout) = self
                .state_changed
                .wait_timeout_while(inner, std::time::Duration::from_secs(2), |state| {
                    state.stream_state == 3 && state.failure.is_none()
                })
                .map_err(|_| io::Error::other("pipewire state lock unavailable"))?;
            inner = state;
            if timeout.timed_out() && inner.stream_state == 3 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PipeWire did not confirm paused stream",
                ));
            }
        }
        if let Some(failure) = &inner.failure {
            return Err(io::Error::other(failure.clone()));
        }
        Ok(())
    }
}

/// The DMA-BUF side of the newest picture (plan 2026-11-14, BK8b). Lock
/// order: `control` before the loop lock; the loop thread never takes
/// `control` and gives buffers back with `queue_locked`; `gpu_live` and the
/// slot are innermost and never held while another lock is taken.
impl LatestFrame {
    pub(super) fn gpu_added(&self, buffer: *mut PwBuffer) {
        let token = self.gpu_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let mut live = self.gpu_live.lock().unwrap_or_else(PoisonError::into_inner);
        live.push((buffer as usize, token));
    }

    pub(super) fn gpu_token(&self, buffer: *mut PwBuffer) -> Option<u64> {
        self.gpu_live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(entry, _)| *entry == buffer as usize)
            .map(|(_, token)| *token)
    }

    pub(super) fn gpu_removed(&self, buffer: *mut PwBuffer) {
        let mut live = self.gpu_live.lock().unwrap_or_else(PoisonError::into_inner);
        live.retain(|(entry, _)| *entry != buffer as usize);
    }

    fn gpu_is_live(&self, buffer: usize, token: u64) -> bool {
        self.gpu_live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(buffer, token))
    }

    /// Gives `(buffer, token)` back to the stream; the caller holds the loop lock.
    pub(super) fn queue_token_locked(&self, buffer: usize, token: u64) {
        let stream = self.native_stream.load(Ordering::Acquire);
        let Some(api) = self.gpu_api.get() else {
            return;
        };
        if !stream.is_null() && self.gpu_is_live(buffer, token) {
            // SAFETY: loop lock held; the exact `(buffer, token)` was dequeued
            // from this stream, is still added (`gpu_live`) and goes back once.
            unsafe { (api.stream_queue_buffer)(stream, buffer as *mut PwBuffer) };
        }
    }

    /// Empties the slot under the loop lock (loop thread or `control`
    /// holder) and gives the picture back directly.
    pub(super) fn clear_gpu_locked(&self) {
        if let Some(held) = self.gpu.clear() {
            let (buffer, token) = held.disarm_token();
            self.queue_token_locked(buffer, token);
        }
    }

    /// Keeps a DMA-BUF picture as the newest one (loop thread). `Err` with
    /// the picture while capture is paused: the caller gives it back.
    pub(super) fn publish_gpu(&self, held: HeldBuffer) -> Result<(), HeldBuffer> {
        if !self.is_capture_active() {
            return Err(held);
        }
        if let Some(older) = self.gpu.publish(held) {
            let (buffer, token) = older.disarm_token();
            self.queue_token_locked(buffer, token);
        }
        Ok(())
    }

    /// The newest DMA-BUF picture for the encoder (merge W2: the source's
    /// `capture_picture` calls this while `dmabuf.dmabuf()` is `Some`,
    /// `LatestFrame::take_gpu_picture`).
    pub(super) fn take_gpu(&self, keyframe: bool) -> io::Result<GpuTaken> {
        self.check_failure()?;
        if !self.is_capture_active() {
            return Ok(GpuTaken::Unchanged);
        }
        Ok(self.gpu.take(keyframe))
    }

    /// Leaves DMA-BUF for the rest of the session from a thread other than
    /// the loop's (the encoder refused the picture or takes CPU pictures,
    /// merge W2: `DmaBufStream` of the GPU picture): renegotiates the
    /// format with the shared memory offer, no new portal dialog.
    pub(super) fn fall_back_from_worker(&self, cause: Fallback) -> io::Result<()> {
        let Some(pods) = self.dmabuf.fall_back(cause) else {
            return Ok(());
        };
        // The session left DMA-BUF whether or not the stream still runs.
        super::dmabuf_stream::report(cause);
        let control = self
            .control
            .lock()
            .map_err(|_| io::Error::other("pipewire pause lock unavailable"))?;
        let Some(control) = control.as_ref() else {
            return Ok(());
        };
        // SAFETY: the owner cannot destroy loop or stream while `control`
        // is locked (detach takes it first).
        unsafe {
            (control.api.thread_loop_lock)(control.thread_loop);
            let stream = self.native_stream.load(Ordering::Acquire);
            let result = super::dmabuf_stream::update_formats(&control.api, stream, &pods);
            self.clear_gpu_locked();
            (control.api.thread_loop_unlock)(control.thread_loop);
            result
        }
    }
}

/// Gives a held DMA-BUF buffer back from the encoder's thread.
pub(super) struct Returner(pub(super) std::sync::Weak<LatestFrame>);

impl BufferReturn for Returner {
    fn give_back(&self, buffer: usize) {
        self.give_back_token(buffer, 0);
    }

    fn give_back_token(&self, buffer: usize, token: u64) {
        let Some(latest) = self.0.upgrade() else {
            return;
        };
        let control = latest
            .control
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(control) = control.as_ref() {
            // SAFETY: as in `fall_back_from_worker`.
            unsafe {
                (control.api.thread_loop_lock)(control.thread_loop);
                latest.queue_token_locked(buffer, token);
                (control.api.thread_loop_unlock)(control.thread_loop);
            }
        }
    }
}
