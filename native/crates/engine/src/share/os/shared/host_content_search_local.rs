//! The local part of a host content search: folders read through held
//! handles on as many threads as the host offers (`agent_proto::ordered_walk`),
//! their files searched in portions any idle thread takes over. A subfolder
//! is opened relative to its held parent without following links, and the
//! host's handle check (`ensure_local_share_handle_allowed`) admits it
//! before anything below it is read; a file is opened relative to its held
//! folder as a regular file only. Every thread builds its own matcher and
//! path filter from the query.
use super::{child_rel, compiled, Shared, Sink, MAX_DEPTH};
use crate::{
    agent_proto::{ContentMatcher, PathFilter},
    local_access::{DirectoryHandle, EntryKind},
    share::{fs_response::FsSkipKind, host_list},
};
use std::{
    ffi::OsString,
    io::{self, Read},
    path::PathBuf,
};

/// Files of one folder handed to a thread at most at once.
const PORTION_FILES: usize = 64;
/// Bytes of the files of one portion after which it is closed.
const PORTION_BYTES: u64 = 8 * 1024 * 1024;

pub(super) enum Job {
    /// A folder to read: its handle, its host path (for the excluded host
    /// storage), its path below the search root and its level.
    Folder(DirectoryHandle, PathBuf, String, usize),
    /// Files of one held folder to search.
    Files(DirectoryHandle, Vec<Item>),
}

/// One listed regular file.
pub(super) struct Item {
    name: OsString,
    rel: String,
    size: u64,
    mtime_ms: i64,
}

/// One thread's output, matcher and path filter.
pub(super) struct Reader<'s, 'a> {
    sink: Sink<'s, 'a>,
    tools: Result<(ContentMatcher, PathFilter), io::Error>,
}

impl crate::agent_proto::WalkOutput for Reader<'_, '_> {
    fn flush(&mut self) -> io::Result<()> {
        self.sink.send()
    }
}

pub(super) fn search(
    shared: &Shared<'_>,
    root: DirectoryHandle,
    physical: PathBuf,
    rel: &str,
    level: usize,
) -> io::Result<()> {
    let excluded = crate::share::storage_roots::excluded();
    let result = crate::agent_proto::ordered_walk(
        vec![Job::Folder(root, physical, rel.to_string(), level)],
        crate::agent_proto::walk_threads(),
        &shared.halt,
        || Reader {
            sink: Sink::new(shared),
            tools: compiled(shared.query),
        },
        |job: Job, local, reader| match job {
            Job::Folder(directory, physical, rel, level) => {
                folder(directory, physical, rel, level, &excluded, local, reader)
            }
            Job::Files(directory, items) => {
                for item in items {
                    file(&directory, item, reader)?;
                }
                Ok(())
            }
        },
    );
    match result {
        Err(_) if shared.ended_at_limit() => Ok(()),
        other => other,
    }
}

fn tools<'r>(
    tools: &'r Result<(ContentMatcher, PathFilter), io::Error>,
) -> io::Result<&'r (ContentMatcher, PathFilter)> {
    tools
        .as_ref()
        .map_err(|error| io::Error::new(error.kind(), error.to_string()))
}

fn folder(
    directory: DirectoryHandle,
    physical: PathBuf,
    rel: String,
    level: usize,
    excluded: &[PathBuf],
    local: &mut crate::agent_proto::WalkLocal<Job>,
    reader: &mut Reader<'_, '_>,
) -> io::Result<()> {
    let shared = reader.sink.shared();
    shared.check()?;
    if shared.limited() {
        return Ok(());
    }
    let (_, filter) = tools(&reader.tools)?;
    let sink = &mut reader.sink;
    if level > MAX_DEPTH {
        sink.skip(
            &rel,
            FsSkipKind::UnlistableFolder,
            "Verzeichnistiefe überschreitet den sicheren Such-Stack",
        );
        return Ok(());
    }
    let entries = match directory.read_directory() {
        Ok(entries) => entries,
        Err(error) => {
            sink.skip(&rel, FsSkipKind::UnlistableFolder, error);
            return Ok(());
        }
    };
    let here = filter.search_in_folder(&rel);
    let mut portion = Vec::new();
    let mut portion_bytes = 0u64;
    for entry in entries {
        if shared.stopped() {
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                sink.skip(&rel, FsSkipKind::UnlistableFolder, error);
                continue;
            }
        };
        let name = entry.name.to_string_lossy();
        if host_list::hidden(&name) || excluded.iter().any(|p| p == &physical.join(&entry.name)) {
            continue;
        }
        // Links and special files are never followed or read.
        if entry.is_link_like || matches!(entry.kind, EntryKind::Link | EntryKind::Other) {
            continue;
        }
        let child = child_rel(&rel, &name);
        let folder = entry.kind == EntryKind::Directory;
        let wanted = if folder {
            shared.query.subfolders && filter.admit_folder(&child, level + 1, &name, entry.hidden)
        } else {
            here && filter.admit_file(&name)
        };
        if !wanted {
            continue;
        }
        if entry.unreachable
            || entry.name.to_str().is_none()
            || crate::vfs::validate_child_name(&name).is_err()
        {
            sink.skip(
                &child,
                FsSkipKind::Unreadable,
                "Name ist kein adressierbarer Share-Pfad",
            );
            continue;
        }
        if folder {
            shared.check()?;
            match directory.open_child(&entry.name) {
                // A folder the host's policy refuses (an unlocked vault) is
                // no Share content: not reported.
                Ok(handle) => {
                    if crate::share::fs::ensure_local_share_handle_allowed(&handle).is_ok() {
                        let path = physical.join(&entry.name);
                        local.push(Job::Folder(handle, path, child, level + 1));
                    }
                }
                Err(error) => sink.skip(&child, FsSkipKind::UnlistableFolder, error),
            }
            continue;
        }
        portion_bytes = portion_bytes.saturating_add(entry.size);
        portion.push(Item {
            name: entry.name,
            rel: child,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
        });
        if portion.len() >= PORTION_FILES || portion_bytes >= PORTION_BYTES {
            local.push(Job::Files(directory.clone(), std::mem::take(&mut portion)));
            portion_bytes = 0;
        }
    }
    if !portion.is_empty() {
        local.push(Job::Files(directory, portion));
    }
    Ok(())
}

/// Searches one listed file, opened as a regular file relative to its held
/// folder after the right was checked again.
fn file(directory: &DirectoryHandle, item: Item, reader: &mut Reader<'_, '_>) -> io::Result<()> {
    let Reader { sink, tools } = reader;
    let shared = sink.shared();
    shared.check()?;
    if shared.limited() {
        return Ok(());
    }
    let (matcher, _) = self::tools(tools)?;
    let opened = match directory.open_regular_child(&item.name) {
        Ok(opened) => opened,
        Err(error) => {
            sink.skip(&item.rel, FsSkipKind::Unreadable, error);
            return Ok(());
        }
    };
    let size = opened.metadata().map_or(item.size, |meta| meta.len());
    let mut input = Watched {
        inner: opened,
        shared,
    };
    sink.begin_file();
    let outcome = crate::agent_proto::search_reader(
        matcher,
        shared.query,
        &item.rel,
        size,
        item.mtime_ms,
        &mut input,
        &mut |event| sink.event(event),
    );
    sink.failure()?;
    match outcome {
        Ok(outcome) => {
            let searched = u64::from(!outcome.skipped);
            shared.totals.searched(searched, outcome.bytes);
            if outcome.skipped && !sink.file_skipped() {
                sink.skip(&item.rel, FsSkipKind::Binary, "");
            }
        }
        Err(error) => {
            // The end of the stream or a revoked right ends the search; the
            // limit ends this file; anything else is this file's failure.
            shared.check()?;
            if !shared.limited() {
                sink.skip(&item.rel, FsSkipKind::Unreadable, error);
            }
        }
    }
    Ok(())
}

/// A file being searched: every read first checks that the search still
/// runs and the session still holds its right.
struct Watched<'s, 'a> {
    inner: std::fs::File,
    shared: &'s Shared<'a>,
}

impl Read for Watched<'_, '_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.shared.stopped() {
            return Err(io::Error::other("Inhaltssuche beendet"));
        }
        self.shared.access.check_read()?;
        self.inner.read(bytes)
    }
}
