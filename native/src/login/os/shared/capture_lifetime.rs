//! Exit and reader end are independent retirement facts. Stop is not either.
use std::{io, sync::{Arc, Mutex}, time::Duration};
use super::{capture_admission::ParentCapture, child::Spawned};

struct State {
    capture: Option<Arc<ParentCapture>>,
    exited: bool,
    reader_done: bool,
    // A failed kill/wait or reaper spawn cannot release outstanding promises.
    retirement: Option<Arc<CaptureLifetime>>,
}
pub(crate) struct CaptureLifetime {state: Mutex<State>}
impl CaptureLifetime {
    pub(crate) fn new(capture: Option<Arc<ParentCapture>>) -> Arc<Self> {
        let owner = Arc::new(Self {state: Mutex::new(State {capture, exited: false, reader_done: false, retirement: None})});
        owner.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retirement = Some(owner.clone());
        owner
    }
    pub(crate) fn capture(&self) -> io::Result<Option<Arc<ParentCapture>>> {
        Ok(self.state.lock().map_err(|_| io::Error::other("login capture retirement poisoned"))?.capture.clone())
    }
    fn advance(&self, exited: bool, reader: bool) {
        let mut state = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.exited |= exited; state.reader_done |= reader;
        if state.exited && state.reader_done {state.capture.take(); state.retirement.take();}
    }
    pub(crate) fn reader(self: &Arc<Self>) -> ReaderEnd {ReaderEnd(self.clone())}
    #[cfg(test)]
    pub(crate) fn confirmed_exit(&self) {self.advance(true, false);}
}
pub(crate) struct ReaderEnd(Arc<CaptureLifetime>);
impl Drop for ReaderEnd {fn drop(&mut self) {self.0.advance(false, true);}}

pub(crate) struct ProcessOwner {spawned: Option<Spawned>, lifetime: Arc<CaptureLifetime>}
impl ProcessOwner {
    pub(crate) fn new(spawned: Spawned, lifetime: Arc<CaptureLifetime>) -> Self {Self {spawned: Some(spawned), lifetime}}
    pub(crate) fn stop(&mut self) {
        let Some(mut process) = self.spawned.take() else {return;};
        if let Ok(Some(capture)) = self.lifetime.capture() {capture.close();}
        if process.life.stop() {self.lifetime.advance(true, false); return;}
        let lifetime = self.lifetime.clone();
        // The armed self-owner remains if spawning fails or a wait panics.
        let _ = std::thread::Builder::new().name("login-capture-retirement".into()).spawn(move || {
            loop {
                if process.life.stop() {lifetime.advance(true, false); break;}
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
}
impl Drop for ProcessOwner {fn drop(&mut self) {self.stop();}}
