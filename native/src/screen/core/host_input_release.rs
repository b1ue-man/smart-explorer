//! One release-only input job retains its original worker while capture is in flight.
use super::*;
use crate::screen::core::wire_control::ControlBlock;
use std::future::Future;

impl Session<'_> {
    pub(super) fn release_pressed(&mut self) {
        // Input is held while this job exists, so a second batch cannot grow behind it.
        if self.input_release.is_some() {return;}
        let released=self.pressed.release_all();
        if released.is_empty() {return;}
        if let Some(sink)=self.pipeline.input_sink() {
            if let Ok(runtime)=tokio::runtime::Handle::try_current() {
                self.input_release=Some(runtime.spawn_blocking(move ||sink.release(&released)));
                return;
            }
        }
        self.release_blocked=self.context.backend.inject(&released).err();
    }
    pub(super) fn poll_input_release(&mut self) {
        let Some(mut job)=self.input_release.take() else {return;};
        let mut cx=std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        match std::pin::Pin::new(&mut job).poll(&mut cx) {
            std::task::Poll::Pending=>self.input_release=Some(job),
            std::task::Poll::Ready(Ok(result))=>self.release_blocked=result.err(),
            std::task::Poll::Ready(Err(_))=>self.release_blocked=Some(ControlBlock::ControlUnavailable),
        }
    }
}
