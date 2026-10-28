//! Requests that reach an SFTP server in the order they were made. A
//! request spawned as a task is sent when the runtime first polls it, so two
//! spawned requests may overtake each other; `enqueue` polls once on the
//! calling thread, which sends the packet, and spawns the rest (G3-06a: a
//! CLOSE behind the last WRITE of a handle is only safe in that order).
use std::future::Future;
use std::task::{Context, Poll};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

/// Starts `request` now: its first poll puts the packet onto the channel
/// (russh-sftp sends in the first poll, before it waits for the answer), so
/// requests reach the server in the order of their `enqueue`. The task then
/// waits for the answer on the runtime.
pub(super) fn enqueue<T, F>(rt: &Runtime, request: F) -> JoinHandle<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let mut request = Box::pin(request);
    let first = {
        // The answer's timeout is a tokio timer: create it in the runtime.
        let _entered = rt.enter();
        let waker = futures_util::task::noop_waker();
        request.as_mut().poll(&mut Context::from_waker(&waker))
    };
    match first {
        Poll::Ready(value) => rt.spawn(async move { value }),
        Poll::Pending => rt.spawn(request),
    }
}
