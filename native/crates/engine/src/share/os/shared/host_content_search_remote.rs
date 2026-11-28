//! The part of a host content search below an exported connection (a remote
//! backend the host reaches itself): the backend's own search
//! (`vfs::content_search`: the agent next to the data, or reading through
//! the backend), behind the export guard that keeps it out of the
//! Share-private names. Its events are checked and forwarded.
//!
//! A connection below the search root (a search from the Share root or the
//! connections folder) sees paths relative to itself: it gets the query
//! without the parts that name folder paths or levels (`folder_include`,
//! `folder_exclude`, first-level `!\` filters), and the host admits each of
//! its events by the whole path below the search root instead.
use super::{child_rel, Sink};
use crate::agent_proto::{ContentEvent, ContentQuery, PathFilter};
use crate::share::fs_response::FsSkipKind;
use crossbeam_channel::RecvTimeoutError;
use std::{io, sync::atomic::Ordering, time::Duration};

/// How often the forwarding thread looks at the stream and the right while
/// the backend is silent.
const POLL: Duration = Duration::from_millis(100);
/// Events the backend may queue ahead of the stream.
const BACKLOG: usize = 64;

pub(super) fn search(
    sink: &mut Sink<'_, '_>,
    filter: &PathFilter,
    backend: &dyn crate::vfs::Backend,
    path: &str,
    rel: &str,
    level: usize,
) -> io::Result<()> {
    let shared = sink.shared();
    let mut query = if rel.is_empty() {
        shared.query.clone()
    } else {
        rebased(shared.query)
    };
    if query.max_hits > 0 {
        query.max_hits = shared.remaining().max(1);
    }
    let (tx, rx) = crossbeam_channel::bounded(BACKLOG);
    let mut seen = (0u64, 0u64);
    let mut stop: Option<io::Error> = None;
    let outcome = std::thread::scope(|scope| {
        let worker =
            scope.spawn(|| crate::vfs::content_search(backend, path, &query, tx, &shared.halt));
        loop {
            if stop.is_none() {
                if let Err(error) = shared.check().and_then(|()| sink.tick()) {
                    shared.halt.store(true, Ordering::Relaxed);
                    stop = Some(error);
                }
            }
            match rx.recv_timeout(POLL) {
                Ok(event) if stop.is_none() => {
                    if let Err(error) = forward(sink, filter, rel, level, event, &mut seen) {
                        shared.halt.store(true, Ordering::Relaxed);
                        stop = Some(error);
                    }
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        worker
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("Verbindungssuche unerwartet beendet")))
    });
    if let Some(error) = stop {
        return if shared.ended_at_limit() {
            Ok(())
        } else {
            Err(error)
        };
    }
    sink.failure()?;
    match outcome {
        Ok(summary) => {
            shared.totals.searched(
                summary.files_searched.saturating_sub(seen.0),
                summary.bytes.saturating_sub(seen.1),
            );
            if summary.limited {
                shared.reach_limit();
            }
            Ok(())
        }
        Err(_) if shared.ended_at_limit() => Ok(()),
        Err(error) => {
            shared.check()?;
            if rel.is_empty() {
                return Err(error);
            }
            sink.skip(rel, FsSkipKind::UnlistableFolder, error);
            Ok(())
        }
    }
}

/// `query` for a connection below the search root: nothing in it names a
/// folder path or level of the search root.
fn rebased(query: &ContentQuery) -> ContentQuery {
    let mut query = query.clone();
    query.folder_include = None;
    query.folder_exclude = None;
    query.filters = query
        .filters
        .split_whitespace()
        .filter(|word| !word.starts_with("!\\"))
        .collect::<Vec<_>>()
        .join(" ");
    query
}

/// One event of the backend: totals into the stream's, others checked,
/// placed below `prefix` and, below the search root, admitted by their
/// whole path.
fn forward(
    sink: &mut Sink<'_, '_>,
    filter: &PathFilter,
    prefix: &str,
    level: usize,
    event: ContentEvent,
    seen: &mut (u64, u64),
) -> io::Result<()> {
    let (rel, folder) = match &event {
        ContentEvent::Progress { files, bytes, .. } => {
            sink.shared()
                .totals
                .searched(files.saturating_sub(seen.0), bytes.saturating_sub(seen.1));
            *seen = ((*files).max(seen.0), (*bytes).max(seen.1));
            return Ok(());
        }
        ContentEvent::Hits { rel, .. } => (rel.as_str(), false),
        ContentEvent::Skipped { rel, reason } => (
            rel.as_str(),
            matches!(reason, crate::agent_proto::SkipReason::UnlistableFolder(_)),
        ),
    };
    if !relative(rel) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Pfad der Verbindungssuche liegt außerhalb der Freigabe",
        ));
    }
    if prefix.is_empty() {
        sink.event(event);
        return Ok(());
    }
    if !admitted(filter, prefix, level, rel, folder) {
        return Ok(());
    }
    let placed = |rel: String| child_rel(prefix, &rel);
    sink.event(match event {
        ContentEvent::Hits {
            rel,
            size,
            mtime_ms,
            lines,
            last,
        } => ContentEvent::Hits {
            rel: placed(rel),
            size,
            mtime_ms,
            lines,
            last,
        },
        ContentEvent::Skipped { rel, reason } => ContentEvent::Skipped {
            rel: placed(rel),
            reason,
        },
        progress @ ContentEvent::Progress { .. } => progress,
    });
    Ok(())
}

/// A path below the backend's root: components only, no `.` or `..`. The
/// root itself (empty) names a folder skip of the root.
fn relative(rel: &str) -> bool {
    rel.is_empty()
        || (!rel.starts_with('/')
            && rel
                .split('/')
                .all(|part| crate::vfs::validate_child_name(part).is_ok()))
}

/// Whether the search root's filter admits `rel` (below the connection at
/// `prefix`, `level`): every folder on its way, and for a file the folder
/// that holds it.
fn admitted(filter: &PathFilter, prefix: &str, level: usize, rel: &str, folder: bool) -> bool {
    let parts: Vec<&str> = rel.split('/').filter(|part| !part.is_empty()).collect();
    let folders = if folder {
        parts.len()
    } else {
        parts.len().saturating_sub(1)
    };
    let mut path = prefix.to_string();
    for (index, name) in parts.iter().take(folders).enumerate() {
        path = child_rel(&path, name);
        if !filter.admit_folder(&path, level + index + 1, name, false) {
            return false;
        }
    }
    folder || filter.search_in_folder(&path)
}
