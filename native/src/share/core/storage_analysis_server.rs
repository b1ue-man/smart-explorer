use crate::analytics::{analysis_transfer::{self, AnalysisMessage}, Progress, ScanPhase};
use iroh::endpoint::SendStream;
use std::io;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Semaphore};

use super::{framing, fs_access::FsAccess, io_deadline, wire::FsResponse};

const HEARTBEAT: Duration = Duration::from_millis(250);

enum Update {
    Control(AnalysisMessage),
    Data(Vec<u8>),
}

struct CancelOnDrop(Progress);
impl Drop for CancelOnDrop {
    fn drop(&mut self) { self.0.cancel.store(true, Ordering::Relaxed); }
}

fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS.get_or_init(|| Arc::new(Semaphore::new(2))).clone()
}

pub(super) async fn serve(mut send: SendStream, root: String, access: FsAccess) -> io::Result<()> {
    let progress = Progress::default();
    let _cancellation = CancelOnDrop(progress.clone());
    progress.set_phase(ScanPhase::Queued, &root);
    let mut ticker = tokio::time::interval(HEARTBEAT);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let stopped = send.stopped();
    tokio::pin!(stopped);
    let acquire = slots().acquire_owned();
    tokio::pin!(acquire);
    let permit = loop {
        tokio::select! {
            permit = &mut acquire => break permit.map_err(io::Error::other)?,
            _ = &mut stopped => return Err(canceled()),
            _ = ticker.tick() => heartbeat(&mut send, &progress).await?,
        }
    };
    progress.set_phase(ScanPhase::Preparing, &root);
    heartbeat(&mut send, &progress).await?;
    let (updates, mut received) = mpsc::channel::<io::Result<Update>>(2);
    let local = progress.clone();
    // Dedicated stack and the same bounded local Rayon pool as a GUI scan.
    // No filesystem traversal or tree encoding occupies the QUIC executor.
    let worker = std::thread::Builder::new()
        .name("share-storage-analytics".into())
        .stack_size(crate::analytics::SCAN_THREAD_STACK_BYTES)
        .spawn(move || {
            let _permit = permit;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_worker(root, access, local, &updates)
            })).unwrap_or_else(|_| Err(io::Error::other("Lokaler Analyse-Worker ist unerwartet beendet")));
            if let Err(error) = result { let _ = updates.blocking_send(Err(error)); }
        })?;
    let mut transferring = false;
    let mut done = false;
    loop {
        tokio::select! {
            _ = &mut stopped => return Err(canceled()),
            _ = ticker.tick(), if !transferring => heartbeat(&mut send, &progress).await?,
            update = received.recv() => {
                match update {
                    Some(Ok(Update::Control(message))) => {
                        transferring |= matches!(&message, AnalysisMessage::Ready { .. });
                        done |= matches!(&message, AnalysisMessage::Done { .. });
                        send_response(&mut send, FsResponse::Analysis { message }).await?;
                    }
                    Some(Ok(Update::Data(bytes))) => {
                        io_deadline::run("analysis result data", framing::send_tagged(
                            &mut send, framing::TAG_DATA, &bytes,
                        )).await?;
                    }
                    Some(Err(error)) => {
                        return send_response(&mut send, super::fs_error::response(&error)).await;
                    }
                    None => break,
                }
            }
        }
    }
    worker.join().map_err(|_| io::Error::other("Analyse-Worker konnte nicht beendet werden"))?;
    if !done { return Err(io::Error::other("Analyse ohne Abschlussmeldung beendet")); }
    Ok(())
}

fn run_worker(root: String, access: FsAccess, progress: Progress, updates: &mpsc::Sender<io::Result<Update>>) -> io::Result<()> {
    progress.check_cancel()?;
    let started = Instant::now();
    let mut outcome = super::storage_analysis_host::scan(&root, &access, &progress);
    let scan_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    progress.check_cancel()?;
    progress.set_phase(ScanPhase::Assembling, &root);
    analysis_transfer::send_outcome(&mut outcome, &progress, Some(scan_ms),
        |message| updates.blocking_send(Ok(Update::Control(message))).map_err(|_| canceled()),
        |bytes| updates.blocking_send(Ok(Update::Data(bytes))).map_err(|_| canceled()),
    )

}

async fn heartbeat(send: &mut SendStream, progress: &Progress) -> io::Result<()> {
    send_response(send, FsResponse::Analysis { message: AnalysisMessage::Progress { state: progress.snapshot() } }).await
}

async fn send_response(send: &mut SendStream, response: FsResponse) -> io::Result<()> {
    io_deadline::run("analysis response", framing::reply(send, response)).await
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Analyse-Verbindung wurde beendet")
}

#[cfg(test)]
#[path = "storage_analysis_task_tests.rs"]
mod task_tests;

#[cfg(all(test, windows))]
#[path = "../os/windows/storage_analysis_task_tests.rs"]
mod windows_task_tests;
