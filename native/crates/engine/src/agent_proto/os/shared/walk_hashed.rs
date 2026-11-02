//! `WalkHashed2` and `WalkHashed3` on the agent's own filesystem, on as many
//! threads as the host offers (`ordered_walk`): every folder and regular
//! file of at least `min_bytes` (with its MD5 when asked), then `End`. Links,
//! special files and unreadable entries are omissions, never a reason to
//! fail the walk; with `meta` (`WalkHashed3`) links and special files come as
//! entries with their listing attributes instead. A failure at the root
//! fails the walk. Every folder is sent before the entries below it.
//! Pseudo filesystems (`is_pseudo_dir`) and the folder of the digest cache
//! (`hash_cache::is_cache_folder`) are a row, as their parent's listing shows
//! them, followed by an omission of the same path: their content is never
//! walked (`walk_kept_out.rs`).
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

use super::fs::systemtime_ms;
use super::ops_types::{digest, omission};
use super::session::{emit, Sink};
use super::walk_keyed::{Keyed, KeyedSetup, Scope};
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
    /// `WalkKeyed`: the folder runs go through the walk's key tree.
    keyed: Option<Keyed<'a>>,
}

impl<'a> Out<'a> {
    fn new(
        sink: &'a Sink,
        id: u64,
        form: Form,
        run: &'a Mutex<()>,
        keys: Option<KeyedSetup<'a>>,
    ) -> Self {
        Self {
            sink,
            id,
            form,
            entries: Vec::new(),
            omitted: Vec::new(),
            bytes: 0,
            run,
            held: None,
            keyed: keys.map(Keyed::new),
        }
    }

    fn meta(&self) -> bool {
        self.form == Form::Parts { meta: true }
    }

    /// The folder `rel` starts (keyed walks track every folder's run).
    fn begin(&mut self, rel: &str) -> io::Result<()> {
        match &mut self.keyed {
            Some(keyed) => keyed.begin(rel),
            None => Ok(()),
        }
    }

    fn entry(&mut self, entry: WireHashEntry) -> io::Result<()> {
        if let Some(keyed) = &mut self.keyed {
            return keyed.entry(entry);
        }
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

    /// A folder the filter leaves out (`walk_filter.rs`): its entry, never
    /// its content.
    fn skipped_dir(&mut self, entry: WireHashEntry) -> io::Result<()> {
        match &mut self.keyed {
            Some(keyed) => keyed.skipped_dir(entry),
            None => self.entry(entry),
        }
    }

    fn omit(&mut self, rel: String, reason: u8, detail: impl Into<String>) -> io::Result<()> {
        let omission = WireOmission {
            rel,
            reason,
            detail: detail.into(),
        };
        if let Some(keyed) = &mut self.keyed {
            return keyed.omit(omission);
        }
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
        if let Some(keyed) = &mut self.keyed {
            return keyed.end(self.sink, self.id);
        }
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

/// The walk of `root` (`digest::NONE` or `digest::MD5`) as `scope` reads
/// and filters it, then `End`.
pub(super) fn walk(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, min_bytes): (u8, u64),
    form: Form,
    scope: Scope<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let roots = vec![(PathBuf::from(root), String::new())];
    let setup = (None, roots, true, scope);
    run_walk(sink, id, (root, algorithm, min_bytes), form, setup, cancel)?;
    emit(sink, id, &Frame::End)
}

/// The walk of `WalkHashed3` with `meta` whose folder runs go through
/// `keys` (`walk_keyed.rs`): of the whole tree, or with `folders` only those
/// folders (paths below `root`), each with its subfolders when `deeper`. The
/// caller sends `End`.
pub(super) fn walk_keyed(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, min_bytes, folders, deeper): (u8, u64, Option<&[String]>, bool),
    keys: KeyedSetup<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let form = Form::Parts { meta: true };
    let roots = match folders {
        None => vec![(PathBuf::from(root), String::new())],
        Some(folders) => {
            let mut roots = Vec::with_capacity(folders.len());
            for rel in folders {
                let path = if rel.is_empty() {
                    PathBuf::from(root)
                } else {
                    super::ValidatedRelativePath::parse(rel)?;
                    Path::new(root).join(rel)
                };
                roots.push((path, rel.clone()));
            }
            roots
        }
    };
    let scope = Scope {
        reads: keys.reads,
        filter: keys.filter,
    };
    let setup = (Some(keys), roots, deeper, scope);
    run_walk(sink, id, (root, algorithm, min_bytes), form, setup, cancel)
}

/// Walk setup: the key tree (if any), the first folders, whether the walk
/// goes below them, and how it reads and filters.
type Setup<'a> = (
    Option<KeyedSetup<'a>>,
    Vec<(PathBuf, String)>,
    bool,
    Scope<'a>,
);

fn run_walk(
    sink: &Sink,
    id: u64,
    (root, algorithm, min_bytes): (&str, u8, u64),
    form: Form,
    (keys, roots, deeper, scope): Setup<'_>,
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
        // A walk collecting in this process serves a local sync side.
        local: keys.is_some_and(|keys| keys.collect.is_some()),
        reads: scope.reads,
        // Only entries with attributes carry the `hidden` the client filters.
        filter: scope.filter.filter(|_| form == Form::Parts { meta: true }),
    };
    let run = Mutex::new(());
    let walked = super::ordered_walk(
        roots,
        super::walk_threads(),
        cancel,
        || Out::new(sink, id, form, &run, keys),
        |(dir, rel), local, out| {
            out.begin(&rel)?;
            if deeper {
                walk.folder(&dir, &rel, local, out)?;
            } else {
                walk.folder(&dir, &rel, &mut WalkLocal::detached(), out)?;
            }
            out.end_folder()
        },
    );
    if algorithm == digest::MD5 {
        super::hash_cache::flush_now();
    }
    walked
}

struct Walk<'a> {
    algorithm: u8,
    min_bytes: u64,
    cancel: &'a AtomicBool,
    /// The walk runs in this process for a local sync side (`local_keyed`):
    /// `hidden` follows the platform's own rule (`walk_keyed::local_hidden`).
    local: bool,
    /// How file contents are read (a fresh read, the run's digests).
    reads: super::LocalReads<'a>,
    /// The job's filter (`walk_filter.rs`): no digest for a file it leaves
    /// out, no walk into a folder it leaves out.
    filter: Option<&'a super::HostFilter>,
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
        // Names with a leading dot are hidden, as SFTP lists them; the walk
        // of a local sync side takes the platform's own rule.
        let hidden = if self.local {
            super::walk_keyed::local_hidden(&name, &metadata)
        } else {
            name.starts_with('.')
        };
        let meta = out.meta().then(|| WireHashMeta {
            hidden,
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
            if self.filter.is_some_and(|f| f.leaves_out_dir(&rel, hidden)) {
                return out.skipped_dir(plain(true, size, None));
            }
            out.entry(plain(true, size, None))?;
            if let Some(detail) = super::walk_kept_out::kept_out_dir(&path) {
                // A row as in the folder's listing; its content is never
                // walked (`walk_kept_out.rs`).
                return out.omit(rel, omission::UNREADABLE, detail);
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
        let left_out = self
            .filter
            .is_some_and(|f| f.leaves_out_file(&rel, hidden, metadata.len(), mtime_ms));
        let md5 = if self.algorithm == digest::MD5 && !left_out {
            let digest = super::md5_cached(&path, &metadata, self.reads, self.cancel, || {
                super::ext_ops::md5_file(&path, metadata.len(), self.cancel)
            });
            match digest {
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
