//! Serialized source activity transitions, without borrowing HostSession while an OS acknowledges.
use super::*;

impl Pipeline {
    pub(crate) fn begin_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.capture_requested = active;
        self.capture_ready = None;
        self.capture_active = false;
        self.invalidate_reference();
        if self.capture_transition.is_some() {
            return Ok(());
        }
        if self.capture_applied == Some(active) {
            self.capture_active = active;
            self.capture_ready = Some(Ok(()));
            return Ok(());
        }
        self.start_capture_transition(active)
    }
    pub(super) fn start_capture_transition(&mut self, active: bool) -> io::Result<()> {
        let runtime = tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        let mut stage = self.stage.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "screen source unavailable")
        })?;
        self.capture_applied = None;
        stage.encoder = None;
        self.capture_transition = Some(runtime.spawn_blocking(move || {
            let result = stage.source.set_capture_active(active);
            (stage, active, result)
        }));
        Ok(())
    }
    pub(crate) fn poll_capture_active(&mut self) -> Option<io::Result<()>> {
        if let Some(result) = self.capture_ready.take() {
            return Some(result);
        }
        let mut job = self.capture_transition.take()?;
        use std::future::Future;
        let mut context = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        let completed = match std::pin::Pin::new(&mut job).poll(&mut context) {
            std::task::Poll::Ready(result) => result,
            std::task::Poll::Pending => {
                self.capture_transition = Some(job);
                return None;
            }
        };
        let (stage, active, result) = match completed {
            Ok(value) => value,
            Err(error) => return Some(Err(io::Error::other(error))),
        };
        self.stage = Some(stage);
        if result.is_ok() {
            self.capture_applied = Some(active);
        }
        if active != self.capture_requested {
            if let Err(error) = self.start_capture_transition(self.capture_requested) {
                return Some(Err(error));
            }
            return None;
        }
        match result {
            Ok(()) => {
                self.capture_active = active;
                if active {
                    self.clock = FrameClock::new(self.quality.fps);
                }
                Some(Ok(()))
            }
            Err(error) => Some(Err(error)),
        }
    }
    pub(crate) fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        if self.capture_transition.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "capture transition pending",
            ));
        }
        self.capture_requested = active;
        self.capture_ready = None;
        if self.capture_applied == Some(active) {
            return Ok(());
        }
        self.capture_applied = None;
        self.capture_active = false;
        self.invalidate_reference();
        let stage = self.stage.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "screen source unavailable")
        })?;
        stage.source.set_capture_active(active)?;
        self.capture_active = active;
        self.capture_applied = Some(active);
        if active {
            self.clock = FrameClock::new(self.quality.fps);
        }
        Ok(())
    }
}
