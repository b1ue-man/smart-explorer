//! `RemoveTree` (`ext-v1`) on the agent's own filesystem: one request deletes
//! a whole folder next to the data instead of one round trip per entry.
//!
//! The folder itself must be a plain directory (never a link). Its children
//! are classified by `DirEntry::file_type`, which does not follow links: a
//! child directory goes to `std::fs::remove_dir_all`, which removes links as
//! entries without following them (and is protected against symlink races
//! on Linux); every other child (file, link, special file) is unlinked. The
//! cancel flag is checked before each child, the counters (top-level entries
//! of the folder plus the folder itself) go out as `Progress` at most every
//! 250 ms with a first and a final report. A root-confined agent stays inside
//! its Landlock root; this file uses the standard library only, so the
//! standalone `se-agent` crate builds it unchanged.
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::session::{emit, Sink};
use super::Frame;

/// Shortest interval between two progress reports.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent folder deletion canceled")
}

/// Throttled `Progress` frames of one request.
struct Reporter<'a> {
    sink: &'a Sink,
    id: u64,
    total: u64,
    last: Option<Instant>,
}

impl Reporter<'_> {
    fn report(&mut self, done: u64, force: bool) -> io::Result<()> {
        let due = self
            .last
            .is_none_or(|last| last.elapsed() >= PROGRESS_EVERY);
        if !force && !due {
            return Ok(());
        }
        self.last = Some(Instant::now());
        emit(
            self.sink,
            self.id,
            &Frame::Progress {
                done,
                total: self.total,
            },
        )
    }
}

/// `RemoveTree`: `Progress`*, then `End`. A failure sends the counters
/// reached as a last `Progress` and returns the error (the request loop
/// answers it with `Err`); a cancel returns `Interrupted` like every other
/// canceled agent request. Entries removed before a cancel or a failure stay
/// removed.
pub(crate) fn handle_remove_tree(
    sink: &Sink,
    id: u64,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let root = Path::new(path);
    let metadata = std::fs::symlink_metadata(root)?;
    if super::local_platform::metadata_is_link_like(root, &metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("zu löschender Ordner ist kein Ordner: {path}"),
        ));
    }
    let children = std::fs::read_dir(root)?.collect::<io::Result<Vec<_>>>()?;
    let mut reporter = Reporter {
        sink,
        id,
        total: (children.len() as u64).saturating_add(1),
        last: None,
    };
    let mut done = 0u64;
    reporter.report(done, true)?;
    for child in children {
        if cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        if let Err(error) = remove_child(&child) {
            // The deletion error is the answer; a lost report changes nothing.
            let _ = reporter.report(done, true);
            return Err(io::Error::new(
                error.kind(),
                format!("{}: {error}", child.path().display()),
            ));
        }
        done = done.saturating_add(1);
        reporter.report(done, false)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    if let Err(error) = std::fs::remove_dir(root) {
        let _ = reporter.report(done, true);
        return Err(error);
    }
    done = done.saturating_add(1);
    reporter.report(done, true)?;
    emit(sink, id, &Frame::End)
}

/// Remove one child of the folder without following it. An entry that
/// vanished meanwhile is already gone.
fn remove_child(child: &std::fs::DirEntry) -> io::Result<()> {
    let path = child.path();
    let kind = child.file_type()?;
    let result = if kind.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        match std::fs::remove_file(&path) {
            // A link to a directory is removed with `remove_dir` on systems
            // that keep directory links apart (Windows junctions and
            // directory symlinks); the link is removed, never its target.
            Err(error) if kind.is_symlink() => std::fs::remove_dir(&path).map_err(|_| error),
            other => other,
        }
    };
    match result {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
