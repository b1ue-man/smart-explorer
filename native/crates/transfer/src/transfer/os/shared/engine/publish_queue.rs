//! Publishing downloaded packet files outside the packet's stream (K-C,
//! 2026-10-28). The sink of `get_batch` hands every finished part to a
//! `Publisher` and goes on reading the stream at network speed, while one
//! publisher thread publishes the parts in arrival order through
//! `download::publish_local_known`: no replacement, keep-both numbering, the
//! receipts and their failure semantics stay exactly those of the single
//! download. One thread keeps the numbering as sequential as before. A part
//! handed over is published even when the stream fails later; what never
//! finished goes alone again, as before. Without a thread the parts are
//! published on the calling thread, in the stream as before. A part handed
//! to a publisher thread that ended without its result (a panic) may have
//! been published: it counts as "outcome unknown", never as "not published",
//! so it is not downloaded a second time next to itself.
#![cfg_attr(not(test), allow(dead_code))]
use super::download::{publish_local_known, Part};
use super::file_evidence::HostEvidence;
use super::ops::{At, OpError, OpResult};
use super::queue::FileWork;
use super::Engine;
use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

/// One finished part of packet member `index`.
struct Task {
    index: usize,
    part: Part,
    destination: PathBuf,
    file: FileWork,
    /// The remote source as the peer described it while sending it: its
    /// receipt then asks the peer for nothing (`publish_local_known`).
    known: Option<HostEvidence>,
}

/// Takes the finished parts of one packet (`with_publisher`).
pub(super) struct Publisher<'p, 'e> {
    engine: &'p Engine<'e>,
    tasks: Option<crossbeam_channel::Sender<Task>>,
    /// Results published on the calling thread (no publisher thread).
    inline: RefCell<Vec<(usize, OpResult)>>,
    /// Members handed to the publisher thread: each one must come back with
    /// a result.
    handed: RefCell<Vec<usize>>,
}

impl Publisher<'_, '_> {
    /// Publishes `part` as `destination` for packet member `index`, on the
    /// publisher thread when there is one. The engine always passes the
    /// stream's evidence (`submit_known`); tests use this short form.
    #[cfg(test)]
    pub(super) fn submit(&self, index: usize, part: Part, destination: PathBuf, file: FileWork) {
        self.submit_known(index, part, destination, file, None);
    }

    /// `submit` with the source evidence the packet's stream carried.
    pub(super) fn submit_known(
        &self,
        index: usize,
        part: Part,
        destination: PathBuf,
        file: FileWork,
        known: Option<HostEvidence>,
    ) {
        let task = Task {
            index,
            part,
            destination,
            file,
            known,
        };
        let task = match &self.tasks {
            Some(tasks) => {
                let index = task.index;
                match tasks.send(task) {
                    Ok(()) => {
                        self.handed.borrow_mut().push(index);
                        return;
                    }
                    Err(refused) => refused.0,
                }
            }
            None => task,
        };
        let result = publish_local_known(
            self.engine,
            task.part,
            &task.destination,
            &task.file,
            task.known,
        );
        self.inline.borrow_mut().push((task.index, result));
    }
}

/// Runs `stream` (the packet's `get_batch`) with a `Publisher`; returns its
/// value and every submitted member's publication result, by member index.
/// Returns only when every submitted part is published.
pub(super) fn with_publisher<'e, T>(
    engine: &Engine<'e>,
    stream: impl FnOnce(&Publisher<'_, 'e>) -> T,
) -> (T, Vec<(usize, OpResult)>) {
    let (tasks, queued) = crossbeam_channel::unbounded::<Task>();
    let (done, results) = crossbeam_channel::unbounded();
    std::thread::scope(|scope| {
        let publisher_thread = std::thread::Builder::new()
            .name("transfer-publish".into())
            .spawn_scoped(scope, move || {
                for task in queued {
                    let result = publish_local_known(
                        engine,
                        task.part,
                        &task.destination,
                        &task.file,
                        task.known,
                    );
                    if done.send((task.index, result)).is_err() {
                        return;
                    }
                }
            });
        let publisher = Publisher {
            engine,
            tasks: publisher_thread.is_ok().then_some(tasks),
            inline: RefCell::new(Vec::new()),
            handed: RefCell::new(Vec::new()),
        };
        let value = stream(&publisher);
        let Publisher {
            tasks,
            inline,
            handed,
            ..
        } = publisher;
        // The publisher thread ends once the last part is published.
        drop(tasks);
        let ended = match publisher_thread {
            Ok(thread) => thread.join().is_ok(),
            Err(_) => true,
        };
        let mut published: Vec<(usize, OpResult)> = results.try_iter().collect();
        unanswered(&mut published, handed.into_inner(), ended);
        published.extend(inline.into_inner());
        (value, published)
    })
}

/// Every handed-over member without a result gets "outcome unknown": the
/// publisher thread ended (a panic) while or before publishing it, and it
/// may already stand under its name.
fn unanswered(published: &mut Vec<(usize, OpResult)>, handed: Vec<usize>, ended: bool) {
    let answered: HashSet<usize> = published.iter().map(|(index, _)| *index).collect();
    let reason = if ended {
        "Veröffentlichung ohne Ergebnis"
    } else {
        "Veröffentlichungs-Thread brach ab"
    };
    for index in handed {
        if !answered.contains(&index) {
            let error = io::Error::other(format!(
                "{reason}: die Datei kann am Ziel bereits stehen und wird nicht erneut geladen"
            ));
            published.push((index, Err(OpError::at(At::Unknown, error))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_publisher_panic_leaves_members_unknown_not_unpublished() {
        let mut published: Vec<(usize, OpResult)> = Vec::new();
        unanswered(&mut published, vec![0, 3], false);
        assert_eq!(published.len(), 2);
        for (_, result) in &published {
            match result {
                Err(failure) => assert_eq!(failure.at, At::Unknown),
                Ok(_) => panic!("a member without result is never published"),
            }
        }
        // Answered members keep their own result.
        let mut answered = vec![(1, Err(OpError::source(io::Error::other("x"))))];
        unanswered(&mut answered, vec![1], true);
        assert_eq!(answered.len(), 1);
        assert!(matches!(&answered[0].1, Err(failure) if failure.at == At::Source));
    }
}
