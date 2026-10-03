//! Long host work uses fair workers and bounded backpressure, never the control pool.
use std::{io, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use iroh::endpoint::SendStream;
use tokio::sync::mpsc;
use crate::share::{analysis_admission, framing, fs_access::FsAccess, io_deadline, session::PeerPrincipal, wire::FsResponse};

pub(in crate::share) struct Stop(pub Arc<AtomicBool>);
impl Drop for Stop { fn drop(&mut self) { self.0.store(true, Ordering::Relaxed); } }

pub(in crate::share) async fn serve(mut send: SendStream, principal: PeerPrincipal,
    authority: FsAccess, cancel: Arc<AtomicBool>, listing: bool, heartbeat: impl Fn() -> FsResponse,
    work: impl FnOnce(mpsc::Sender<io::Result<FsResponse>>, Arc<AtomicBool>) -> io::Result<()> + Send + 'static,
) -> io::Result<()> {
    let _stop = Stop(cancel.clone());
    authority.register_cancel(&cancel)?;
    let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
    let ticket = if listing { analysis_admission::listing() } else { analysis_admission::host() }.enqueue(principal);
    let acquire = ticket.acquire(); tokio::pin!(acquire);
    let stopped = send.stopped(); tokio::pin!(stopped);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let permit = loop { tokio::select! {
        permit = &mut acquire => break permit,
        _ = &mut stopped => return Err(io::ErrorKind::Interrupted.into()),
        _ = tick.tick() => {
            authority.check_read()?;
            io_deadline::run("Share queued heartbeat", framing::reply(&mut send, heartbeat())).await?;
        }
    } };
    let (tx, mut rx) = mpsc::channel(2);
    let worker_cancel = cancel.clone();
    std::thread::Builder::new().name("share-host-walk".into()).stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
        .spawn(move || {
            let _permit = permit;
            let _awake = crate::keep_awake::hold(crate::keep_awake::Reason::PeerService);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(tx.clone(), worker_cancel)))
                .unwrap_or_else(|_| Err(io::Error::other("Host-Walk unerwartet beendet")));
            if let Err(error) = result { let _ = tx.blocking_send(Err(error)); }
        })?;
    loop { tokio::select! {
        _ = &mut stopped => return Err(io::ErrorKind::Interrupted.into()),
        _ = tick.tick() => {
            authority.check_read()?;
            io_deadline::run("Share walk heartbeat", framing::reply(&mut send, heartbeat())).await?;
        }
        next = rx.recv() => match next {
            Some(Ok(response)) => {
                authority.check_read()?;
                io_deadline::run("Share walk portion", framing::reply(&mut send, response)).await?;
            }
            Some(Err(error)) => return framing::reply_err(&mut send, error).await,
            None => { send.finish().map_err(io::Error::other)?; return Ok(()); }
        }
    } }
}
pub(in crate::share) fn emit(tx: &mpsc::Sender<io::Result<FsResponse>>, cancel: &AtomicBool, response: FsResponse) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) { return Err(io::ErrorKind::Interrupted.into()); }
    tx.blocking_send(Ok(response)).map_err(|_| io::ErrorKind::Interrupted.into())
}
