//! `WalkHashed2` and `WalkHashed3` on the agent's own filesystem, on as many
//! threads as the host offers (`ordered_walk`): every folder and regular
//! file of at least `min_bytes` (with its MD5 when asked), then `End`. Links,
//! special files and unreadable entries are omissions, never a reason to
//! fail the walk; with `meta` (`WalkHashed3`) links and special files come as
//! entries with their listing attributes instead. A failure at the root
//! fails the walk. Every folder is sent before the entries below it.
//! Pseudo filesystems (`is_pseudo_dir`) are a row, as their parent's listing
//! shows them, followed by an omission of the same path: their content is
//! never walked.
//!
//! `WalkHashed2` answers one `HashEntry`/`HashOmitted` frame per item, as
//! before; `WalkHashed3` answers `HashPart`s, whose items keep the order they
//! were produced in (a part never holds an entry after an omission, the
//! client forwards the entries of a part before its omissions). With `meta`
//! the items of one folder (its entries and omissions, and the omission of
//! the folder itself) form one unbroken run after the folder's own entry:
//! a folder that fills more than one part holds the walk's run lock until
//! its last part is sent, so no other thread's part comes in between.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use super::fs::{is_pseudo_dir, systemtime_ms};
use super::ops_types::{digest, omission};
use super::session::{emit, Sink};
use super::{Frame, WalkLocal, WalkOutput, WireHashEntry, WireHashMeta, WireOmission};

/// Estimated encoded bytes after which a `HashPart` is sent.
const PART_BYTES: usize = 256 * 1024;

/// How the items of one walk travel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Form {
    /// `WalkHashed2`: one frame per item.
    Single,
    /// `WalkHashed3`: portions; `meta` = entries carry listing attributes.
    Parts { meta: bool },
}

struct Out<'a> {
    sink: &'a Sink,
    id: u64,
    form: Form,
    entries: Vec<WireHashEntry>,
    omitted: Vec<WireOmission>,
    bytes: usize,
    /// Shared by every thread of the walk; every part is sent under it.
    run: &'a Mutex<()>,
    /// Held from a part sent in the middle of a folder (`meta`) until that
    /// folder's last part is out.
    held: Option<MutexGuard<'a, ()>>,
}

impl<'a> Out<'a> {
    fn new(sink: &'a Sink, id: u64, form: Form, run: &'a Mutex<()>) -> Self {
        Self {
            sink,
            id,
            form,
            entries: Vec::new(),
            omitted: Vec::new(),
            bytes: 0,
            run,
            held: None,
        }
    }

    fn meta(&self) -> bool {
        self.form == Form::Parts { meta: true }
    }

    fn entry(&mut self, entry: WireHashEntry) -> io::Result<()> {
        if self.form == Form::Single {
            return emit(
                self.sink,
                self.id,
                &Frame::HashEntry {
                    rel: entry.rel,
                    is_dir: entry.is_dir,
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                    md5: entry.digest,
                },
            );
        }
        // The client forwards a part's entries before its omissions.
        if !self.omitted.is_empty() {
            self.send_within()?;
        }
        self.bytes += entry.rel.len() + entry.digest.as_ref().map_or(0, String::len) + 40;
        self.entries.push(entry);
        self.full()
    }

    fn omit(&mut self, rel: String, reason: u8, detail: impl Into<String>) -> io::Result<()> {
        let omission = WireOmission {
            rel,
            reason,
            detail: detail.into(),
        };
        if self.form == Form::Single {
            return emit(self.sink, self.id, &Frame::HashOmitted(omission));
        }
        self.bytes += omission.rel.len() + omission.detail.len() + 16;
        self.omitted.push(omission);
        self.full()
    }

    fn full(&mut self) -> io::Result<()> {
        if self.bytes >= PART_BYTES {
            self.send_within()?;
        }
        Ok(())
    }

    /// Sends a part while the current folder is not done yet; with `meta`
    /// the run lock stays held until `end_folder`.
    fn send_within(&mut self) -> io::Result<()> {
        if self.meta() && self.held.is_none() {
            self.held = Some(lock(self.run));
        }
        self.send_part()
    }

    /// The current folder is done: a run lock held for it is released.
    fn end_folder(&mut self) -> io::Result<()> {
        if self.held.is_some() {
            return WalkOutput::flush(self);
        }
        Ok(())
    }

    fn send_part(&mut self) -> io::Result<()> {
        self.bytes = 0;
        if self.entries.is_empty() && self.omitted.is_empty() {
            return Ok(());
        }
        emit(
            self.sink,
            self.id,
            &Frame::HashPart {
                entries: std::mem::take(&mut self.entries),
                omitted: std::mem::take(&mut self.omitted),
            },
        )
    }
}

fn lock(run: &Mutex<()>) -> MutexGuard<'_, ()> {
    run.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl WalkOutput for Out<'_> {
    /// Called between folders: sends the buffered parts under the run lock.
    fn flush(&mut self) -> io::Result<()> {
        let result = if self.held.is_some() || (self.entries.is_empty() && self.omitted.is_empty())
        {
            self.send_part()
        } else {
            let _run = lock(self.run);
            self.send_part()
        };
        self.held = None;
        result
    }
}

/// The omission code of a failure while reading one entry.
fn failure_reason(error: &io::Error) -> u8 {
    if error.kind() == io::ErrorKind::NotFound {
        omission::VANISHED
    } else {
        omission::UNREADABLE
    }
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent hash walk canceled")
}

/// The walk of `root` (`digest::NONE` or `digest::MD5`), then `End`.
pub(super) fn walk(
    sink: &Sink,
    id: u64,
    root: &str,
    algorithm: u8,
    min_bytes: u64,
    form: Form,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !matches!(algorithm, digest::NONE | digest::MD5) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unbekannter Hashalgorithmus",
        ));
    }
    let base = Path::new(root);
    let metadata = std::fs::symlink_metadata(base)?;
    if super::local_platform::metadata_is_link_like(base, &metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Wurzel ist kein Ordner: {root}"),
        ));
    }
    let walk = Walk {
        algorithm,
        min_bytes,
        cancel,
    };
    let run = Mutex::new(());
    super::ordered_walk(
        vec![(base.to_path_buf(), String::new())],
        super::walk_threads(),
        cancel,
        || Out::new(sink, id, form, &run),
        |(dir, rel), local, out| {
            walk.folder(&dir, &rel, local, out)?;
            out.end_folder()
        },
    )?;
    emit(sink, id, &Frame::End)
}

struct Walk<'a> {
    algorithm: u8,
    min_bytes: u64,
    cancel: &'a AtomicBool,
}

impl Walk<'_> {
    fn folder(
        &self,
        dir: &Path,
        rel_dir: &str,
        local: &mut WalkLocal<(PathBuf, String)>,
        out: &mut Out<'_>,
    ) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        // Re-check queued children: a link inserted after the parent's
        // listing is an omission, never a directory traversal outside it.
        let current = match std::fs::symlink_metadata(dir) {
            Ok(current) => current,
            Err(error) if rel_dir.is_empty() => return Err(error),
            Err(error) => {
                return out.omit(rel_dir.into(), failure_reason(&error), error.to_string())
            }
        };
        if super::local_platform::metadata_is_link_like(dir, &current) {
            return out.omit(rel_dir.into(), omission::LINK, "Verknüpfung");
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if rel_dir.is_empty() => return Err(error),
            Err(error) => {
                return out.omit(rel_dir.into(), failure_reason(&error), error.to_string())
            }
        };
        for entry in entries {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            let entry = match entry {
                Ok(entry) => entry,
                // The enumeration of this folder broke off: the rest of it
                // is unknown, never absent.
                Err(error) => {
                    return out.omit(rel_dir.into(), omission::UNREADABLE, error.to_string())
                }
            };
            self.entry(entry, rel_dir, local, out)?;
        }
        Ok(())
    }

    fn entry(
        &self,
        entry: std::fs::DirEntry,
        rel_dir: &str,
        local: &mut WalkLocal<(PathBuf, String)>,
        out: &mut Out<'_>,
    ) -> io::Result<()> {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if rel_dir.is_empty() {
            name.clone()
        } else {
            format!("{rel_dir}/{name}")
        };
        if entry.file_name().to_str().is_none() {
            return out.omit(
                rel,
                omission::UNREPRESENTABLE,
                "Name ist kein gültiges UTF-8",
            );
        }
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => return out.omit(rel, failure_reason(&error), error.to_string()),
        };
        let (link, special) = super::local_platform::metadata_class(&path, &metadata);
        let mtime_ms = metadata.modified().ok().map(systemtime_ms).unwrap_or(0);
        // Names with a leading dot are hidden, as SFTP lists them.
        let meta = out.meta().then(|| WireHashMeta {
            hidden: name.starts_with('.'),
            system: false,
            btime_ms: metadata.created().ok().map(systemtime_ms).unwrap_or(0),
            is_symlink: link,
            special: !link && (special || !(metadata.is_dir() || metadata.is_file())),
        });
        let plain = |is_dir: bool, size: u64, digest: Option<String>| WireHashEntry {
            rel: rel.clone(),
            is_dir,
            size,
            mtime_ms,
            digest,
            meta,
        };
        if link {
            if meta.is_some() {
                return out.entry(plain(false, metadata.len(), None));
            }
            return out.omit(rel, omission::LINK, "Verknüpfung");
        }
        if metadata.is_dir() {
            // With attributes a folder has the size its listing reports.
            let size = if meta.is_some() { metadata.len() } else { 0 };
            out.entry(plain(true, size, None))?;
            if path.to_str().is_some_and(is_pseudo_dir) {
                // A row as in the folder's listing; its content (kernel and
                // device state, no files) is never walked.
                return out.omit(
                    rel,
                    omission::UNREADABLE,
                    "Pseudo-Dateisystem: Inhalt wird nicht durchlaufen",
                );
            }
            local.push((path, rel));
            return Ok(());
        }
        if special || !metadata.is_file() {
            if meta.is_some() {
                return out.entry(plain(false, 0, None));
            }
            return out.omit(rel, omission::SPECIAL, "Pipe, Socket oder Gerät");
        }
        if metadata.len() < self.min_bytes {
            return Ok(());
        }
        let md5 = if self.algorithm == digest::MD5 {
            match super::ext_ops::md5_file(&path, metadata.len(), self.cancel) {
                Ok(md5) => Some(md5),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => return out.omit(rel, failure_reason(&error), error.to_string()),
            }
        } else {
            None
        };
        out.entry(plain(false, metadata.len(), md5))
    }
}
