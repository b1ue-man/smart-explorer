//! Pause shared by portal variants; retains the granted stream and rejects expired sources.
use super::*;

#[derive(Default)]
struct LatestInner {
    seq: u64,
    taken: u64,
    image: Option<BgraImage>,
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
    state_changed: std::sync::Condvar,
}

impl LatestFrame {
    fn lock(&self) -> MutexGuard<'_, LatestInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn publish(&self, image: BgraImage) {
        if !self.active.load(Ordering::Acquire) {return;}
        let mut inner = self.lock();
        inner.seq += 1;
        inner.image = Some(image);
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
        self.lock().failure.get_or_insert(reason);
        self.state_changed.notify_all();
    }

    /// A picture newer than the last one taken; else `Unchanged`, but with
    /// `keyframe` the last picture again (a key frame always has one).
    pub(crate) fn take(&self, keyframe: bool) -> Taken {
        let mut inner = self.lock();
        if let Some(reason) = &inner.failure {
            return Taken::Failed(reason.clone());
        }
        if !self.active.load(Ordering::Acquire) { return Taken::Unchanged; }
        let fresh = inner.seq != inner.taken;
        inner.taken = inner.seq;
        match &inner.image {
            Some(image) if fresh || keyframe => Taken::Frame(image.clone()),
            _ => Taken::Unchanged,
        }
    }
}


impl Default for LatestFrame {
    fn default() -> Self { Self {inner:Mutex::default(),trace:Trace::default(),control:Mutex::new(None),active:std::sync::atomic::AtomicBool::new(true),state_changed:std::sync::Condvar::new()} }
}
struct Control {api:Arc<PipewireApi>,thread_loop:*mut c_void,stream:*mut c_void}
// SAFETY: access is serialized by control and PipeWire's thread-loop lock; detach precedes destruction.
unsafe impl Send for Control {}
impl LatestFrame {
    pub(super) fn is_capture_active(&self) -> bool {self.active.load(Ordering::Acquire)}
    pub(super) fn on_state_changed(&self,old:c_int,new:c_int) {
        self.lock().stream_state=new;
        if new==0 && old>=2 {self.fail("pipewire_stream_disconnected".into());}
        self.state_changed.notify_all();
    }
    pub(super) fn suspend_before_connect(&self) {self.active.store(false,Ordering::Release);}
    pub(super) fn attach(&self,api:Arc<PipewireApi>,thread_loop:*mut c_void,stream:*mut c_void) {
        *self.control.lock().unwrap_or_else(PoisonError::into_inner)=Some(Control {api,thread_loop,stream});
    }
    pub(super) fn detach(&self) {
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
                let result=(control.api.stream_set_active)(control.stream,active);
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
