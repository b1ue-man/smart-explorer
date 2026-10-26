//! Pause shared by portal variants; retains the granted stream and rejects expired sources.
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
pub(crate) struct LatestFrame {
    inner: Mutex<LatestInner>,
    pub(super) trace: Trace,
    control: Mutex<Option<Control>>,
    active: std::sync::atomic::AtomicBool,
    admitted: std::sync::atomic::AtomicBool,
    imports_blocked:std::sync::atomic::AtomicBool,
    state_changed: std::sync::Condvar,
    failed:tokio::sync::Notify,
    native_stream:AtomicPtr<c_void>,
}

impl LatestFrame {
    fn lock(&self) -> MutexGuard<'_, LatestInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn publish(&self, image: CapturedImage) {
        if !self.is_capture_active() {return;}
        let mut inner = self.lock();
        inner.seq += 1;
        inner.image = Some(image);
    }

    pub(crate) fn check_failure(&self)->io::Result<()> {
        match &self.lock().failure {Some(reason)=>Err(io::Error::other(reason.clone())),None=>Ok(())}
    }
    /// What the stream did so far (the failure text of a silent stream).
    #[cfg(test)]
    pub(crate) fn describe(&self) -> String {
        let pictures = self.lock().seq;
        format!("{pictures} pictures; {}", self.trace.describe())
    }

    /// The stream runs, but every buffer it sent so far was an empty or
    /// corrupted chunk: it only reports cursor moves, no screen change (the
    /// source sends a picture only when the screen changes).
    #[cfg(test)]
    pub(crate) fn only_empty_chunks(&self) -> bool {
        self.lock().seq == 0 && self.trace.only_empty_chunks()
    }

    pub(crate) fn fail(&self, reason: String) {
        self.admitted.store(false,Ordering::Release);
        self.lock().failure.get_or_insert(reason);
        self.failed.notify_one();
        self.state_changed.notify_all();
    }

    pub(crate) async fn wait_failure(&self) {
        loop {
            let notified=self.failed.notified();
            if self.lock().failure.is_some() {return;}
            notified.await;
        }
    }
    /// A picture newer than the last one taken; else `Unchanged`, but with
    /// `keyframe` the last picture again (a key frame always has one).
    pub(crate) fn take(&self, keyframe: bool) -> Taken {
        let mut inner = self.lock();
        if let Some(reason) = &inner.failure {
            return Taken::Failed(reason.clone());
        }
        if !self.is_capture_active() { return Taken::Unchanged; }
        let fresh = inner.seq != inner.taken;
        inner.taken = inner.seq;
        match &inner.image {
            Some(image) if fresh || keyframe => Taken::Frame(image.clone()),
            _ => Taken::Unchanged,
        }
    }
}


impl Default for LatestFrame {
    fn default() -> Self { Self {inner:Mutex::default(),trace:Trace::default(),control:Mutex::new(None),active:std::sync::atomic::AtomicBool::new(true),admitted:std::sync::atomic::AtomicBool::new(true),imports_blocked:std::sync::atomic::AtomicBool::new(false),state_changed:std::sync::Condvar::new(),failed:tokio::sync::Notify::new(),native_stream:AtomicPtr::new(null_mut())} }
}
struct Control {api:Arc<PipewireApi>,thread_loop:*mut c_void}
// SAFETY: access is serialized by control and PipeWire's thread-loop lock; detach precedes destruction.
unsafe impl Send for Control {}
impl LatestFrame {
    pub(super) fn is_capture_active(&self) -> bool {self.active.load(Ordering::Acquire) && self.admitted.load(Ordering::Acquire) && !self.import_blocked()}
    pub(super) fn on_state_changed(&self,old:c_int,new:c_int) {
        self.lock().stream_state=new;
        if new==0 && old>=2 {self.fail("pipewire_stream_disconnected".into());}
        self.state_changed.notify_all();
    }
    pub(super) fn block_import(&self,blocked:bool) {self.imports_blocked.store(blocked,Ordering::Release);}
    pub(super) fn import_blocked(&self)->bool {self.imports_blocked.load(Ordering::Acquire)}
    pub(super) fn close_copy(&self) {
        self.active.store(false,Ordering::Release);self.admitted.store(false,Ordering::Release);
        self.lock().image=None;
    }
    pub(crate) fn set_initial_active(&self,active:bool) {self.active.store(active,Ordering::Release);}
    pub(super) fn suspend_before_connect(&self) {
        self.admitted.store(false,Ordering::Release);
    }
    pub(super) fn pending_format(&self) {
        self.admitted.store(false,Ordering::Release);
        let mut inner=self.lock();inner.image=None;inner.taken=inner.seq;
    }
    // The caller holds the loop lock and already owns the exact stream.
    // No control mutex is taken: set_capture_active takes that before loop.
    pub(super) fn admitted_locked(&self) {self.admitted.store(true,Ordering::Release);}
    pub(super) fn desired_active(&self)->bool {self.active.load(Ordering::Acquire)}
    pub(super) fn attach(&self,api:Arc<PipewireApi>,thread_loop:*mut c_void,stream:*mut c_void) {
        self.native_stream.store(stream,Ordering::Release);
        *self.control.lock().unwrap_or_else(PoisonError::into_inner)=Some(Control {api,thread_loop});
    }
    pub(super) fn stream_destroyed(&self) {self.native_stream.store(null_mut(),Ordering::Release);}
    pub(super) fn detach(&self) {
        self.stream_destroyed();
        self.control.lock().unwrap_or_else(PoisonError::into_inner).take();
        self.active.store(false,Ordering::Release); self.fail("pipewire_stream_closed".into());
    }
    pub(crate) fn set_capture_active(&self,active:bool) -> io::Result<()> {
        let control=self.control.lock().map_err(|_|io::Error::other("pipewire pause lock unavailable"))?;
        let failure=self.lock().failure.clone();
        if active { if let Some(failure)=failure {return Err(io::Error::other(failure));} }
        if let Some(control)=control.as_ref() {
            // SAFETY: owner cannot destroy these objects while control is locked.
            let result=unsafe {
                (control.api.thread_loop_lock)(control.thread_loop);
                self.active.store(active,Ordering::Release);
                {let mut inner=self.lock();inner.image=None;inner.taken=inner.seq;}
                let stream=self.native_stream.load(Ordering::Acquire);
                let result=if stream.is_null() {-libc::ENOTCONN}else {(control.api.stream_set_active)(stream,active)};
                if result<0 {self.active.store(false,Ordering::Release);}
                (control.api.thread_loop_unlock)(control.thread_loop); result
            };
            if result<0 {return Err(io::Error::other(format!("pw_stream_set_active failed: {result}")));}
        } else {
            self.active.store(active,Ordering::Release);
            let mut inner=self.lock();inner.image=None;inner.taken=inner.seq;
        }
        let mut inner=self.lock();
        if !active {
            let (state,timeout)=self.state_changed.wait_timeout_while(inner,std::time::Duration::from_secs(2),
                |state|state.stream_state==3 && state.failure.is_none()).map_err(|_|io::Error::other("pipewire state lock unavailable"))?;
            inner=state;
            if timeout.timed_out() && inner.stream_state==3 {return Err(io::Error::new(io::ErrorKind::TimedOut,"PipeWire did not confirm paused stream"));}
        }
        if let Some(failure)=&inner.failure {return Err(io::Error::other(failure.clone()));}
        Ok(())
    }
}
