//! Packets of small files for peers that speak a batch protocol (Share hosts,
//! the SSH agent): one round trip for many files. The packet size follows
//! the measured rate (≈ 250 ms per packet) within the backend's limits.
//! A member of a failed packet goes alone again only when it certainly was
//! not published, a peer that is too busy is waited for, and one packet
//! feeds the breaker at most once, however many members it takes along.
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

/// More small files for a packet with `file`, or `None` for a single
/// transfer (no batch support, a large file, nothing else small queued).
pub(super) fn members(engine: &Engine<'_>, file: &FileWork) -> Option<Vec<FileWork>> {
    let limits = engine.batch?;
    if file.retried || file.alone || engine.resume {
        return None;
    }
    let (target, small) = {
        let sizer = super::lock(&engine.sizer);
        (
            sizer.target_bytes(limits.max_bytes),
            sizer.small_file_limit(limits.max_bytes),
        )
    };
    if file.size > small {
        return None;
    }
    let more = engine.queue.take_small(
        limits.max_files.saturating_sub(1),
        target.saturating_sub(file.size),
        small,
    );
    (!more.is_empty()).then_some(more)
}

/// Runs one packet of `first` and `members`.
pub(super) fn run(engine: &Engine<'_>, first: FileWork, members: Vec<FileWork>, buffer: &mut [u8]) {
    let files: Vec<FileWork> = std::iter::once(first)
        .chain(members)
        .filter(|file| parent_ready(engine, file).is_some())
        .collect();
    match (engine.view.source, engine.view.target) {
        (Side::Local, Side::Remote(target)) => {
            super::batch_put::upload(engine, target, files, buffer)
        }
        (Side::Remote(source), Side::Local) => {
            super::batch_get::download(engine, source, files, buffer)
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
