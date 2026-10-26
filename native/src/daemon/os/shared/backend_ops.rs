//! Extension frames (`ext-v1`) the background service answers for the
//! backend it serves (a Share peer): each through the `vfs` extension calls,
//! so the peer's host does the work where it can (duplicate search, hash
//! walk, recycling, stage times, change notices) and the documented fallback
//! applies where it cannot.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::agent::ext_wire::{
    change_to_wire, durability_from_wire, group_to_wire, limits_to_wire, omission_to_wire,
    progress_to_wire, summary_to_wire, unsupported,
};
use crate::agent_proto::{query, Frame, UNSUPPORTED_EXTENSION};
use crate::vfs::{self as vfs, BackendHandle, ChangeNotice, RecycleExpectation, RecycleOutcome};

use super::backend_server::{emit, Sink};

/// How often a running duplicate search reports its counters.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Shortest polling interval a change subscription may ask for.
const MIN_POLL: Duration = Duration::from_secs(1);

fn unsupported_reply(sink: &Sink, id: u64) -> io::Result<()> {
    emit(sink, id, &Frame::Err(UNSUPPORTED_EXTENSION.to_string()))
}

/// `ListTolerant`: the backend's tolerant listing in parts, then `End`.
pub(super) fn list_tolerant(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    path: &str,
) -> io::Result<()> {
    let listing = vfs::list_dir_tolerant(&**backend, path)?;
    crate::agent_proto::emit_listing_parts(
        listing.entries.into_iter().map(crate::agent::vfs_to_wire),
        listing.omitted.into_iter().map(omission_to_wire),
        |part| emit(sink, id, &part),
    )?;
    emit(sink, id, &Frame::End)
}

/// `FindDuplicates`: the storing host searches; its counters every 250 ms,
/// then the groups, the summary and `End`. A cancel of the request reaches
/// the host's search at once.
pub(super) fn find_duplicates(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    min_bytes: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !vfs::supports_duplicate_search(&**backend, root)? {
        return unsupported_reply(sink, id);
    }
    let progress = crate::analytics::ReclaimProgress::default();
    let mut reporting = Ok(());
    let outcome = std::thread::scope(|scope| {
        let worker = scope.spawn(|| vfs::find_duplicates(&**backend, root, min_bytes, &progress));
        while !worker.is_finished() {
            if cancel.load(Ordering::Relaxed) {
                progress.cancel.store(true, Ordering::Relaxed);
            }
            if reporting.is_ok() {
                reporting = emit(sink, id, &Frame::DupProgress(progress_to_wire(&progress)));
                if reporting.is_err() {
                    // The client is gone: stop the host's search.
                    progress.cancel.store(true, Ordering::Relaxed);
                }
            }
            std::thread::sleep(PROGRESS_EVERY);
        }
        worker.join()
    });
    reporting?;
    let report = match outcome {
        Ok(Ok(Some(report))) => report,
        Ok(Ok(None)) => return unsupported_reply(sink, id),
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(io::Error::other("duplicate search of the peer stopped")),
    };
    for group in report.groups {
        crate::agent_proto::emit_duplicate_parts(group_to_wire(group), |part| {
            emit(sink, id, &part)
        })?;
    }
    let summary = summary_to_wire(report.summary, report.root_error);
    emit(sink, id, &Frame::DupSummary(summary))?;
    emit(sink, id, &Frame::End)
}

/// `Query`: what the served backend offers below `path`.
pub(super) fn query(backend: &BackendHandle, kind: u8, path: &str) -> io::Result<Frame> {
    let yes = |value: bool| u64::from(value);
    let value = match kind {
        query::DUPLICATE_SEARCH => yes(vfs::supports_duplicate_search(&**backend, path)?),
        query::HASH_WALK => yes(vfs::supports_hash_walk(&**backend, path)?),
        query::RECYCLE => yes(vfs::supports_recycle(&**backend, path)?),
        query::CHANGE_SIGNAL => match vfs::change_signal_mode(&**backend, path)? {
            Some(vfs::ChangeSignalMode::Push) => 1,
            Some(vfs::ChangeSignalMode::Poll) => 2,
            None => 0,
        },
        query::SYNC_FILESYSTEM => yes(vfs::sync_filesystem(&**backend, path)?),
        query::REMOVE_TREE => yes(vfs::supports_remove_tree(&**backend, path)?),
        _ => 0,
    };
    Ok(Frame::Answer(value))
}

/// `Recycle`: the host re-checks the content and moves the file to its trash.
pub(super) fn recycle(
    backend: &BackendHandle,
    path: &str,
    size: u64,
    sha256: Option<String>,
) -> io::Result<Frame> {
    let expected = RecycleExpectation { size, sha256 };
    match vfs::recycle(&**backend, path, &expected) {
        Ok(RecycleOutcome::Recycled) => Ok(Frame::Answer(1)),
        Ok(RecycleOutcome::Changed) => Ok(Frame::Answer(2)),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Err(unsupported()),
        Err(error) => Err(error),
    }
}

/// `FinishStage`: time, mode and durability of a complete stage.
pub(super) fn finish_stage(
    backend: &BackendHandle,
    stage: &str,
    mtime_ms: Option<i64>,
    mode: Option<u32>,
    durability: u8,
) -> io::Result<Frame> {
    let finish = vfs::StageFinish {
        mtime_ms,
        mode,
        durability: durability_from_wire(durability),
    };
    let finished = vfs::finish_stage(&**backend, stage, finish)?;
    Ok(Frame::StageDone {
        mtime_applied: finished.mtime_applied,
        durable: finished.durable,
    })
}

/// `TargetLimits`: what the target below `root` can store.
pub(super) fn target_limits(backend: &BackendHandle, root: &str) -> Frame {
    Frame::Limits(limits_to_wire(vfs::target_limits(&**backend, root)))
}

/// `Watch`: the backend's change notices until the client cancels or the
/// subscription ends (then a last `Change` with its reason and `End`).
pub(super) fn watch(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    root: &str,
    poll_ms: u64,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (tx, rx) = crossbeam_channel::unbounded();
    let interval = Duration::from_millis(poll_ms).max(MIN_POLL);
    let Some(subscription) = vfs::change_signal(&**backend, root, interval, tx)? else {
        return unsupported_reply(sink, id);
    };
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(notice) => {
                let ended = matches!(notice, ChangeNotice::Ended(_));
                emit(sink, id, &Frame::Change(change_to_wire(notice)))?;
                if ended {
                    break Ok(());
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                let notice = ChangeNotice::Ended("Änderungs-Abo der Gegenstelle beendet".into());
                emit(sink, id, &Frame::Change(change_to_wire(notice)))?;
                break Ok(());
            }
        }
    };
    drop(subscription);
    result.and_then(|()| emit(sink, id, &Frame::End))
}

/// Typed capacity payload, queried only on the authorized backend.
pub(super) fn capacity(backend: &BackendHandle, root: &str) -> io::Result<Frame> {
    let value = vfs::capacity(&**backend, root)?;
    serde_json::to_string(&value).map(Frame::CapacityData)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "capacity serialization failed"))
}

/// Folder failures preserve typed provider classes while retaining the
/// service's existing congestion, transport and locked-vault envelopes.
pub(super) fn folder_reply(result: io::Result<Frame>, credit: bool) -> Frame {
    match result {
        Err(error) => {
            let text = super::backend_server::error_text(&error, credit);
            if text != error.to_string() { Frame::Err(text) }
            else { Frame::folder_reply(Err(error)) }
        }
        Ok(frame) => frame,
    }
}

pub(super) fn folder_operation(backend: &BackendHandle, request: Frame, credit: bool) -> Frame {
    let result = match request {
        Frame::RenameDirectoryNoReplace { src, dst } => {
            vfs::rename_reserved_directory(&**backend, &src, &dst).map(|()| Frame::Ok)
        }
        Frame::CreateDirExclusive(path) => {
            vfs::create_dir_exclusive(&**backend, &path).map(|()| Frame::Ok)
        }
        Frame::DirectoryReservationSupported(root) => {
            Ok(Frame::Answer(u64::from(vfs::directory_reservation_supported(&**backend, &root))))
        }
        Frame::DirectoryIdentity(path) => {
            vfs::directory_identity(&**backend, &path).map(Frame::DirectoryIdentityValue)
        }
        Frame::FileIdentity(path) => {
            // Keep the backend's own guards and opt-in. Stat IDs may be
            // content hashes, paths or provider IDs without file semantics.
            vfs::file_identity(&**backend, &path).map(Frame::FileIdentityValue)
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "Unexpected folder request")),
    };
    folder_reply(result, credit)
}

#[cfg(test)]
mod reservation_compatibility_tests {
    use super::*;
    use crate::vfs::{Backend, BackendExtensions, Scheme, VfsMeta};
    use std::{io::{Read, Write}, sync::{Arc, atomic::{AtomicUsize, Ordering}}};

    #[derive(Default)]
    struct Provider { ordinary: AtomicUsize, strict: AtomicUsize }
    impl Backend for Provider {
        fn scheme(&self) -> Scheme { Scheme::Webdav }
        fn root_display(&self) -> String { "/".into() }
        fn extensions(&self) -> Option<&dyn BackendExtensions> { Some(self) }
        fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> { Ok(Vec::new()) }
        fn stat(&self, _: &str) -> io::Result<VfsMeta> { Ok(VfsMeta::default()) }
        fn open_read(&self, _: &str) -> io::Result<Box<dyn Read + Send>> { Err(io::Error::from(io::ErrorKind::Unsupported)) }
        fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { Err(io::Error::from(io::ErrorKind::Unsupported)) }
        fn rename(&self, _: &str, _: &str) -> io::Result<()> { self.ordinary.fetch_add(1, Ordering::SeqCst); Ok(()) }
        fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> { self.rename(source, destination) }
        fn remove_file(&self, _: &str) -> io::Result<()> { Err(io::Error::from(io::ErrorKind::Unsupported)) }
        fn remove_dir(&self, _: &str) -> io::Result<()> { Err(io::Error::from(io::ErrorKind::Unsupported)) }
        fn mkdir_all(&self, _: &str) -> io::Result<()> { Err(io::Error::from(io::ErrorKind::Unsupported)) }
    }
    impl BackendExtensions for Provider {
        fn rename_reserved_directory(&self, _: &str, _: &str) -> io::Result<()> {
            self.strict.fetch_add(1, Ordering::SeqCst); Err(io::Error::new(io::ErrorKind::Unsupported, "no atomic binding rename"))
        }
    }

    #[test]
    fn batches_task_transfer_compat_daemon_reserved_frame_keeps_strict_provider_boundary() {
        let provider = Arc::new(Provider::default()); let backend: BackendHandle = provider.clone();
        let reply = folder_operation(&backend, Frame::RenameDirectoryNoReplace { src: "/owned".into(), dst: "/final".into() }, true);
        let Frame::Err(message) = reply else { panic!("strict unsupported was lost"); };
        assert_eq!(Frame::folder_error(&message).unwrap().kind(), io::ErrorKind::Unsupported);
        assert_eq!(provider.strict.load(Ordering::SeqCst), 1); assert_eq!(provider.ordinary.load(Ordering::SeqCst), 0);
        backend.rename_no_replace("/old", "/new").unwrap();
        assert_eq!(provider.ordinary.load(Ordering::SeqCst), 1);
    }
}
