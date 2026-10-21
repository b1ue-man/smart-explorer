//! Two-stage copy between two open handles: one thread reads ahead into a
//! ring of blocks, the calling thread writes them, so a slow read and a slow
//! write overlap instead of alternating. No platform imports: the handles are
//! plain `Read` and `Write`.
// Only `windows.rs` uses it outside the tests; other hosts check the logic.
#![cfg_attr(not(windows), allow(dead_code))]
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError,
};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// How often the writing thread looks at the cancel flag while it waits for
/// the reader: a stalled source must not hold a cancel back.
const CANCEL_POLL: Duration = Duration::from_millis(20);

/// The side of a copy an error belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipeSide {
    Read,
    Write,
}

#[derive(Debug)]
pub(crate) struct PipeError {
    pub side: PipeSide,
    pub error: io::Error,
}

/// What the reading thread hands to the writer.
enum Item {
    Block(Vec<u8>, usize),
    End,
    Failed(io::Error),
}

/// Copies `reader` to its end into `writer`. A thread reads (a ring of
/// `depth` blocks of `block` bytes circulates between it and the caller),
/// the calling thread writes and is the only one that calls `progress`
/// (after each block) and looks at `cancel` (between blocks). Returns the
/// bytes written, or `None` when `cancel` was set. When the thread cannot
/// be started the caller copies alone, block by block.
pub(crate) fn pipe_copy(
    reader: &mut (dyn Read + Send),
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
    depth: usize,
) -> Result<Option<u64>, PipeError> {
    let block = block.max(1);
    let depth = depth.max(1);
    let (full_tx, full_rx) = sync_channel::<Item>(depth);
    let (empty_tx, empty_rx) = channel::<Vec<u8>>();
    // Raised when the writer ends early so the reader stops reading ahead.
    let stop = AtomicBool::new(false);
    // The reader stays with the caller when the thread cannot be started.
    let reader = Mutex::new(reader);
    std::thread::scope(|scope| {
        let spawned = std::thread::Builder::new()
            .name("copy-read".to_string())
            .spawn_scoped(scope, {
                let reader = &reader;
                let stop = &stop;
                move || {
                    let mut reader = lock(reader);
                    read_ahead(&mut **reader, block, depth, &empty_rx, &full_tx, stop);
                }
            });
        match spawned {
            Ok(_) => write_blocks(full_rx, empty_tx, writer, cancel, &stop, progress),
            Err(_) => {
                let mut reader = lock(&reader);
                copy_alone(&mut **reader, writer, cancel, progress, block)
            }
        }
    })
}

fn lock<'a, 'r>(
    reader: &'a Mutex<&'r mut (dyn Read + Send)>,
) -> MutexGuard<'a, &'r mut (dyn Read + Send)> {
    reader
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The reading thread: fills free blocks of the ring and sends them in
/// order; ends after the end of the input, an error or when the writer left.
fn read_ahead(
    reader: &mut (dyn Read + Send + '_),
    block: usize,
    depth: usize,
    empty: &Receiver<Vec<u8>>,
    full: &SyncSender<Item>,
    stop: &AtomicBool,
) {
    crate::transfer::lower_thread_priority();
    let mut allocated = 0usize;
    loop {
        // Blocks are allocated when first needed, so a small file never pays
        // for the whole ring.
        let mut buffer = match empty.try_recv() {
            Ok(buffer) => buffer,
            Err(TryRecvError::Empty) if allocated < depth => {
                allocated += 1;
                vec![0u8; block]
            }
            Err(TryRecvError::Empty) => match empty.recv() {
                Ok(buffer) => buffer,
                Err(_) => return,
            },
            Err(TryRecvError::Disconnected) => return,
        };
        if stop.load(Ordering::Acquire) {
            return;
        }
        let item = loop {
            match reader.read(&mut buffer) {
                Ok(0) => break Item::End,
                Ok(read) => break Item::Block(buffer, read),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => break Item::Failed(error),
            }
        };
        let last = !matches!(item, Item::Block(..));
        if full.send(item).is_err() || last {
            return;
        }
    }
}

/// The calling thread: writes the blocks in order and gives them back. It
/// owns both ends it uses, so the reader is released the moment it returns.
fn write_blocks(
    full: Receiver<Item>,
    empty: Sender<Vec<u8>>,
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<Option<u64>, PipeError> {
    let mut copied = 0u64;
    let outcome = loop {
        if cancel.load(Ordering::Acquire) {
            break Ok(None);
        }
        match full.recv_timeout(CANCEL_POLL) {
            Ok(Item::Block(buffer, length)) => {
                if let Err(error) = writer.write_all(&buffer[..length]) {
                    break Err(PipeError {
                        side: PipeSide::Write,
                        error,
                    });
                }
                copied = copied.saturating_add(length as u64);
                progress(length as u64);
                // The reader may be gone already; the block is then dropped.
                let _ = empty.send(buffer);
            }
            Ok(Item::End) => break Ok(Some(copied)),
            Ok(Item::Failed(error)) => {
                break Err(PipeError {
                    side: PipeSide::Read,
                    error,
                })
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                break Err(PipeError {
                    side: PipeSide::Read,
                    error: io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "copy reader thread ended unexpectedly",
                    ),
                })
            }
        }
    };
    stop.store(true, Ordering::Release);
    outcome
}

/// One block at a time on the calling thread (no reader thread available).
fn copy_alone(
    reader: &mut (dyn Read + Send + '_),
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
    block: usize,
) -> Result<Option<u64>, PipeError> {
    let mut buffer = vec![0u8; block];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let read = match reader.read(&mut buffer) {
            Ok(0) => return Ok(Some(copied)),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(PipeError {
                    side: PipeSide::Read,
                    error,
                })
            }
        };
        writer
            .write_all(&buffer[..read])
            .map_err(|error| PipeError {
                side: PipeSide::Write,
                error,
            })?;
        copied = copied.saturating_add(read as u64);
        progress(read as u64);
    }
}

#[cfg(test)]
#[path = "pipe_copy_tests.rs"]
mod tests;
