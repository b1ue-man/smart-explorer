//! Bounded provider streams keep backpressure and cancellation at the guard.
use std::{io, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use crossbeam_channel::{bounded, Receiver, Sender, RecvTimeoutError, SendTimeoutError};
use super::super::{fs_access::AccessAuthority, fs_policy::TargetPolicy};

const TICK: Duration = Duration::from_millis(50);
const BACKLOG: usize = 16;

pub(super) fn cancellable<T: Send>(authority: Option<&Arc<AccessAuthority>>, external: &AtomicBool,
    produce: impl FnOnce(&AtomicBool) -> io::Result<T> + Send) -> io::Result<T> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(authority) = authority { authority.register_cancel(&cancel)?; }
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker_cancel = cancel.clone();
        let worker = scope.spawn(move || { let _ = tx.send(produce(&worker_cancel)); });
        let mut failure = None;
        let produced = loop {
            if failure.is_none() {
                if let Err(error) = check(authority, external, &cancel) {
                    failure = Some(error);
                    cancel.store(true, Ordering::Release);
                }
            }
            match rx.recv_timeout(TICK) {
                Ok(result) => break result,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err(io::Error::other("Share-Provider-Worker beendet")),
            }
        };
        worker.join().map_err(|_| io::Error::other("Share-Provider-Worker beendet"))?;
        if let Some(error) = failure { return Err(error); }
        check(authority, external, &cancel)?;
        produced
    })
}

pub(super) fn forward<T: Send>(authority: Option<&Arc<AccessAuthority>>, policy: &TargetPolicy,
    root: &str, tx: Sender<T>, external_cancel: &AtomicBool,
    relative: impl Fn(&T) -> &str,
    produce: impl FnOnce(Sender<T>, &AtomicBool) -> io::Result<bool> + Send) -> io::Result<bool> {
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(authority) = authority { authority.register_cancel(&cancel)?; }
    let (from_provider, incoming) = bounded(BACKLOG);
    std::thread::scope(|scope| {
        let worker_cancel = cancel.clone();
        let worker = scope.spawn(move || produce(from_provider, &worker_cancel));
        let result = drain(authority, policy, root, incoming, tx, external_cancel, &cancel, relative);
        cancel.store(true, Ordering::Release);
        let produced = worker.join().map_err(|_| io::Error::other("Share-Provider-Worker beendet"))?;
        result?;
        produced
    })
}

fn drain<T>(authority: Option<&Arc<AccessAuthority>>, policy: &TargetPolicy, root: &str,
    incoming: Receiver<T>, tx: Sender<T>, external_cancel: &AtomicBool, cancel: &AtomicBool,
    relative: impl Fn(&T) -> &str) -> io::Result<()> {
    loop {
        check(authority, external_cancel, cancel)?;
        let item = match incoming.recv_timeout(TICK) {
            Ok(item) => item,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let rel = relative(&item);
        validate_relative(rel)?;
        let path = format!("{}/{}", root.trim_end_matches('/'), rel);
        if !policy.visible(&path, false) { continue; }
        let mut item = item;
        loop {
            check(authority, external_cancel, cancel)?;
            match tx.send_timeout(item, TICK) {
                Ok(()) => break,
                Err(SendTimeoutError::Timeout(pending)) => item = pending,
                Err(SendTimeoutError::Disconnected(_)) => return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted, "Share-Empfänger beendet")),
            }
        }
    }
}
fn validate_relative(rel: &str) -> io::Result<()> {
    if rel.is_empty() { return Ok(()); }
    if rel.starts_with('/') { return Err(io::Error::new(io::ErrorKind::InvalidData, "Provider-Pfad liegt außerhalb der Freigabe")); }
    for part in rel.split('/') { crate::vfs::validate_child_name(part)?; }
    Ok(())
}
fn check(authority: Option<&Arc<AccessAuthority>>, external: &AtomicBool, local: &AtomicBool) -> io::Result<()> {
    if external.load(Ordering::Acquire) || local.load(Ordering::Acquire) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Share-Provider-Lauf abgebrochen"));
    }
    if let Some(authority) = authority { authority.check()?; }
    Ok(())
}

struct Watch {
    cancelled: Arc<AtomicBool>,
    underlying: Option<crate::vfs::ChangeSubscription>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        drop(self.underlying.take());
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

pub(super) fn subscribe(authority: Option<Arc<AccessAuthority>>, policy: TargetPolicy, root: String,
    tx: Sender<crate::vfs::ChangeNotice>,
    start: impl FnOnce(Sender<crate::vfs::ChangeNotice>) -> io::Result<Option<crate::vfs::ChangeSubscription>>)
    -> io::Result<Option<crate::vfs::ChangeSubscription>> {
    let cancelled = Arc::new(AtomicBool::new(false));
    if let Some(authority) = &authority { authority.register_cancel(&cancelled)?; }
    let (output, incoming) = bounded(BACKLOG);
    let Some(underlying) = start(output)? else { return Ok(None) };
    let mut guard = Watch { cancelled: cancelled.clone(), underlying: Some(underlying), worker: None };
    let worker = std::thread::Builder::new().name("share-guarded-notices".into()).spawn(move || {
        while !cancelled.load(Ordering::Acquire) {
            if authority.as_ref().is_some_and(|auth| auth.check().is_err()) { return; }
            let mut item = match incoming.recv_timeout(TICK) {
                Ok(item) => item,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if let crate::vfs::ChangeNotice::Changed { paths, .. } = &mut item {
                let had_paths = !paths.is_empty();
                paths.retain(|rel| validate_relative(rel).is_ok() && policy.visible(
                    &format!("{}/{}", root.trim_end_matches('/'), rel), false));
                if had_paths && paths.is_empty() { continue; }
            }
            if let crate::vfs::ChangeNotice::Ended(message) = &mut item {
                if super::super::fs_policy::private_path(message) { *message = "Share-Beobachtung beendet".into(); }
            }
            loop {
                if cancelled.load(Ordering::Acquire) || authority.as_ref().is_some_and(|auth| auth.check().is_err()) { return; }
                match tx.send_timeout(item, TICK) {
                    Ok(()) => break,
                    Err(SendTimeoutError::Timeout(pending)) => item = pending,
                    Err(SendTimeoutError::Disconnected(_)) => return,
                }
            }
        }
    })?;
    guard.worker = Some(worker);
    Ok(Some(crate::vfs::ChangeSubscription::new(guard)))
}
