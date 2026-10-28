//! Publishing downloaded packet files outside the packet's stream (K-C,
//! 2026-10-28). The sink of `get_batch` hands every finished part to a
//! `Publisher` and goes on reading the stream at network speed, while one
//! publisher thread publishes the parts in arrival order through
//! `download::publish_local`: no replacement, keep-both numbering, the
//! receipts and their failure semantics stay exactly those of the single
//! download. One thread keeps the numbering as sequential as before. A part
//! handed over is published even when the stream fails later; what never
//! finished goes alone again, as before. Without a thread the parts are
//! published on the calling thread, in the stream as before.
#![cfg_attr(not(test), allow(dead_code))]
use super::download::{publish_local, Part};
use super::ops::OpResult;
use super::queue::FileWork;
use super::Engine;
use std::cell::RefCell;
use std::path::PathBuf;

/// One finished part of packet member `index`.
struct Task {
    index: usize,
    part: Part,
    destination: PathBuf,
    file: FileWork,
}

/// Takes the finished parts of one packet (`with_publisher`).
pub(super) struct Publisher<'p, 'e> {
    engine: &'p Engine<'e>,
    tasks: Option<crossbeam_channel::Sender<Task>>,
    /// Results published on the calling thread (no publisher thread).
    inline: RefCell<Vec<(usize, OpResult)>>,
}

impl Publisher<'_, '_> {
    /// Publishes `part` as `destination` for packet member `index`, on the
    /// publisher thread when there is one.
    pub(super) fn submit(&self, index: usize, part: Part, destination: PathBuf, file: FileWork) {
        let task = Task {
            index,
            part,
            destination,
            file,
        };
        let task = match &self.tasks {
            Some(tasks) => match tasks.send(task) {
                Ok(()) => return,
                Err(refused) => refused.0,
            },
            None => task,
        };
        let result = publish_local(self.engine, task.part, &task.destination, &task.file);
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
                    let result = publish_local(engine, task.part, &task.destination, &task.file);
                    if done.send((task.index, result)).is_err() {
                        return;
                    }
                }
            });
        let publisher = Publisher {
            engine,
            tasks: publisher_thread.is_ok().then_some(tasks),
            inline: RefCell::new(Vec::new()),
        };
        let value = stream(&publisher);
        let Publisher { tasks, inline, .. } = publisher;
        // The publisher thread ends once the last part is published.
        drop(tasks);
        if let Ok(thread) = publisher_thread {
            let _ = thread.join();
        }
        let mut published: Vec<(usize, OpResult)> = results.try_iter().collect();
        published.extend(inline.into_inner());
        (value, published)
    })
}
