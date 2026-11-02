//! One release-only input job retains its original worker while capture is in flight.
use super::*;
use crate::screen::core::wire_control::ControlBlock;
use std::future::Future;

/// Runs one input injection on tokio's blocking pool and waits for it. A
/// host process answers input only after a running picture request (the
/// worker sink waits up to `INPUT_TIMEOUT` for the link, then as long for
/// the answer); tokio 1.52.3: "issuing a blocking call ... in a future
/// without yielding is problematic, as it may prevent the executor from
/// driving other futures forward", `spawn_blocking` "runs the provided
/// closure on a thread dedicated to blocking operations". The session still
/// waits for the result, so input keeps its order and its control state.
pub(super) async fn inject_off_runtime(
    inject: impl FnOnce() -> Result<(), ControlBlock> + Send + 'static,
) -> Result<(), ControlBlock> {
    tokio::task::spawn_blocking(inject)
        .await
        .unwrap_or(Err(ControlBlock::ControlUnavailable))
}

impl Session<'_> {
    pub(super) fn release_pressed(&mut self) {
        // Input is held while this job exists, so a second batch cannot grow behind it.
        if self.input_release.is_some() {
            return;
        }
        let released = self.pressed.release_all();
        if released.is_empty() {
            return;
        }
        if let Some(sink) = self.pipeline.input_sink() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                self.input_release = Some(runtime.spawn_blocking(move || sink.release(&released)));
                return;
            }
        }
        self.release_blocked = self.context.backend.inject(&released).err();
    }
    pub(super) fn poll_input_release(&mut self) {
        let Some(mut job) = self.input_release.take() else {
            return;
        };
        let mut cx = std::task::Context::from_waker(futures_util::task::noop_waker_ref());
        match std::pin::Pin::new(&mut job).poll(&mut cx) {
            std::task::Poll::Pending => self.input_release = Some(job),
            std::task::Poll::Ready(Ok(result)) => self.release_blocked = result.err(),
            std::task::Poll::Ready(Err(_)) => {
                self.release_blocked = Some(ControlBlock::ControlUnavailable)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inject_off_runtime;
    use crate::screen::core::wire_control::ControlBlock;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    /// Input that a host process holds back behind a running picture
    /// request blocks a blocking-pool thread, not the session's runtime:
    /// on a current-thread runtime the task that ends the wait still runs
    /// and the input then arrives. A panicking platform injection becomes a
    /// blocked control instead of a dead session task.
    #[test]
    fn screen_metadata_runtime_task_input_injection_leaves_the_runtime_free() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            let waiting = gate.clone();
            let injected = inject_off_runtime(move || {
                let (open, changed) = &*waiting;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
                Ok(())
            });
            let opener = tokio::spawn(async move {
                let (open, changed) = &*gate;
                *open.lock().unwrap() = true;
                changed.notify_all();
            });
            let result = tokio::time::timeout(Duration::from_secs(10), injected).await;
            assert_eq!(result.expect("the runtime kept running"), Ok(()));
            opener.await.unwrap();
            let failed = inject_off_runtime(|| panic!("platform input panicked")).await;
            assert_eq!(failed, Err(ControlBlock::ControlUnavailable));
        });
    }
}
