//! Packets of small files for peers that speak a batch protocol (Share hosts,
//! the SSH agent): one round trip for many files. The packet size follows
//! the measured rate (≈ 250 ms per packet) within the backend's limits.
//! A member of a failed packet goes alone again only when it certainly was
//! not published, a peer that is too busy is waited for, and one packet
//! feeds the breaker at most once, however many members it takes along.
//! Tiny files fill a packet up to the backend's file count (K-E); a resumed
//! job packs every file without a receipt of its own, and "transfer missing
//! files" packs downloads, whose packet asks `existing` per member. Small
//! local copies of a job with a crash log travel in local packets: one
//! intent record and one permit for many files (`local_packet`, M3).
use super::super::engine_names::parent_rel;
use super::super::engine_policy::{ends_job_at_target, is_back_pressure, is_transient};
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::ops::{At, OpError};
use super::queue::{FileWork, Work};
use super::view::Side;
use super::worker::{
    breaker, is_connection_failure, overload_wait, parent_ready, report, run_file, target_refuses,
};
use super::Engine;
use std::time::Duration;

#[path = "local_packet.rs"]
mod local_packet;

/// More small files for a packet with `file`, or `None` for a single
/// transfer (no batch support, a large file, nothing else small queued).
pub(super) fn members(engine: &Engine<'_>, file: &FileWork) -> Option<Vec<FileWork>> {
    if local_packet::applies(engine) {
        return local_packet::members(engine, file);
    }
    let limits = engine.batch?;
    if super::batch_remote_server::refused(engine) {
        return None;
    }
    if file.retried || file.alone || !resume_allows(engine, file) || !packet_source(engine, file) {
        return None;
    }
    let (target, small) = {
        let sizer = super::lock(&engine.sizer);
        (
            sizer.packet_bytes(limits.max_bytes),
            sizer.small_file_limit(limits.max_bytes),
        )
    };
    if file.size > small {
        return None;
    }
    let candidates = engine.queue.take_small(
        limits.max_files.saturating_sub(1),
        target.saturating_sub(file.size),
        small,
    );
    admit(engine, candidates)
}

/// The candidates a packet keeps; the others go back to the queue as single
/// files before any packet reader can take their permit.
fn admit(engine: &Engine<'_>, candidates: Vec<FileWork>) -> Option<Vec<FileWork>> {
    let mut more = Vec::with_capacity(candidates.len());
    let mut single = Vec::new();
    // The first SSD member does not authorize any later member's medium.
    // Probe after releasing the queue mutex; optical/probe-error entries go
    // back as single files before any packet reader can acquire their permit.
    for mut candidate in candidates {
        if resume_allows(engine, &candidate) && packet_source(engine, &candidate) {
            more.push(candidate);
        } else {
            candidate.alone = true;
            single.push(candidate);
        }
    }
    for candidate in single.into_iter().rev() {
        engine.queue.push_front(Work::File(candidate));
    }
    (!more.is_empty()).then_some(more)
}

/// Whether a resumed job may send `file` in a packet: a file with a receipt
/// of this job is decided alone (`file_recovery::resume`), and "transfer
/// missing files" keeps an existing destination, which only the download
/// packet checks per member (`batch_get` asks `existing`); uploads go alone
/// there. A new job packs everything.
fn resume_allows(engine: &Engine<'_>, file: &FileWork) -> bool {
    let resumed = engine.view.recovery == crate::transfer::RecoveryMode::Resume;
    if !resumed && !engine.resume {
        return true;
    }
    let download = !engine.view.source.is_local() && engine.view.target.is_local();
    (!engine.resume || download) && !super::file_recovery::has_receipt(engine, file)
}

/// A local member must not be on an optical medium. Discovery queues only
/// regular files, so the cached per-folder probe applies (M6): no volume
/// resolution per member on Windows.
fn packet_source(engine: &Engine<'_>, file: &FileWork) -> bool {
    if !engine.view.source.is_local() {
        return true;
    }
    let path = super::super::walk_listers::native(&file.source);
    crate::source_media::probe_regular_file(&path).is_ok_and(|source| source.is_none())
}

/// Runs one packet of `first` and `members`. The members' folders that do
/// not exist yet are created first in one request where the target creates
/// folders in bulk (`FolderRegister::ensure_many`), instead of one request
/// per new folder when each member asks for its parent.
pub(super) fn run(engine: &Engine<'_>, first: FileWork, members: Vec<FileWork>, buffer: &mut [u8]) {
    let files: Vec<FileWork> = std::iter::once(first).chain(members).collect();
    let mut parents: Vec<&str> = files
        .iter()
        .filter_map(|file| parent_rel(&file.rel))
        .collect();
    parents.sort_unstable();
    parents.dedup();
    if !parents.is_empty() {
        engine.folders.ensure_many(&parents, engine.stop_flag());
    }
    let files: Vec<FileWork> = files
        .into_iter()
        .filter(|file| parent_ready(engine, file).is_some())
        .collect();
    match (engine.view.source, engine.view.target) {
        (Side::Local, Side::Remote(target)) => {
            super::batch_put::upload(engine, target, files, buffer)
        }
        (Side::Remote(source), Side::Local) => {
            super::batch_get::download(engine, source, files, buffer)
        }
        (Side::Remote(source), Side::Remote(target)) => {
            super::batch_remote::copy(engine, source, target, files, buffer)
        }
        (Side::Local, Side::Local) if local_packet::applies(engine) => {
            local_packet::run(engine, files, buffer)
        }
        _ => {
            for file in files {
                run_file(engine, file, buffer);
            }
        }
    }
}

/// How far a failed packet member got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Attempt {
    /// Never tried: the packet ended before its bytes went out.
    Untried,
    /// Tried and certainly not published (a local publication that did not
    /// happen, bytes that were refused or discarded).
    NotPublished,
    /// The peer may have published it: reported, never sent again.
    MaybePublished,
}

/// The failed members of one packet, decided one by one.
pub(super) struct Failures<'e, 'a> {
    engine: &'e Engine<'a>,
    /// The packet's one connection failure was counted.
    counted: bool,
    /// Refused as too busy: sent again after the longest wait they asked for.
    waiting: Vec<FileWork>,
    delay: Duration,
}

impl<'e, 'a> Failures<'e, 'a> {
    pub(super) fn new(engine: &'e Engine<'a>) -> Self {
        Self {
            engine,
            counted: false,
            waiting: Vec::new(),
            delay: Duration::ZERO,
        }
    }

    pub(super) fn member(&mut self, mut file: FileWork, failure: OpError, attempt: Attempt) {
        let engine = self.engine;
        if attempt != Attempt::MaybePublished {
            if let Err(error) = super::file_recovery::unpublished(engine, &file) {
                engine.fatal(format!(
                    "Unveröffentlichten Dateibeleg zurücknehmen: {error}"
                ));
                return;
            }
        }
        if engine.stopped() {
            return;
        }
        let kind = failure.error.kind();
        if failure.at == At::Target && ends_job_at_target(kind) {
            engine.fatal(target_refuses(&failure.error));
            return;
        }
        let overload = classify_error(&failure.error) == OpOutcome::Overload;
        let refused = attempt != Attempt::MaybePublished || failure.at != At::Unknown;
        if refused && is_back_pressure(kind, overload) {
            // A refusal publishes nothing (K13): the file waits for its turn.
            if let Some(delay) = overload_wait(engine, &mut file, &failure.error) {
                self.delay = self.delay.max(delay);
                self.waiting.push(file);
                return;
            }
        } else {
            match attempt {
                Attempt::Untried => {
                    file.alone = true;
                    engine.queue.push_front(Work::File(file));
                    return;
                }
                Attempt::NotPublished if !file.retried && is_transient(kind, overload) => {
                    file.retried = true;
                    engine.queue.push_front(Work::File(file));
                    return;
                }
                Attempt::NotPublished | Attempt::MaybePublished => {}
            }
        }
        report(engine, &file.source, &failure);
        if !self.counted && is_connection_failure(&failure) {
            self.counted = true;
            breaker(engine, &failure.error);
        }
    }

    /// Sends the refused members again once the peer's delay has passed
    /// (without any permit; a stop drops them).
    pub(super) fn finish(self) {
        if self.waiting.is_empty() || !super::sleep_unless(self.engine.stop_flag(), self.delay) {
            return;
        }
        for file in self.waiting {
            self.engine.queue.push_front(Work::File(file));
        }
    }
}
