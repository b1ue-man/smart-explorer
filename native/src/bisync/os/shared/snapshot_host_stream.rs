//! One snapshot side from a walk next to the data with listing attributes
//! (`HashWalkRequest::meta`: Share `hash_walk_meta_v1`, agent and service
//! `walk-meta-v1`), checked while it streams. Such a walk sends the items of
//! one folder (its entries and omissions, and the omission of the folder
//! itself) as one unbroken run after that folder's own entry, every folder
//! before the items below it. So a folder is complete as soon as an item of
//! another folder arrives. It is then checked with `scan_listing` exactly as
//! the walk over listings checks it (filters, hidden entries, protected
//! names, links, special files, budget, duplicate names), its files go into
//! the tree, `ctx.listed` learns of it (early copies start) and the folders
//! `scan_listing` enters are expected next.
//!
//! Only the folder in progress is held, never the whole tree. Items below a
//! folder `scan_listing` does not enter (filters, hidden, own or system
//! folders) or below a folder that failed are dropped as they arrive and
//! never counted. Folders without items (empty ones, ones that could not be
//! opened) are checked when the walk ends. An item outside this order fails
//! the side, so a misordered walk can never become a snapshot.
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::Ordering;

use super::snapshot_dir::{
    literal_child, record_listing_omissions, record_omission, scan_listing, WalkContext,
};
use super::snapshot_walk::{merge_files, MAX_WALK_DEPTH};
use super::types::Tree;
use crate::vfs::{HashWalkEntry, HashWalkItem, OmissionReason, VfsMeta, VfsOmission};

/// A folder `scan_listing` entered whose run has not ended yet.
struct Expected {
    path: String,
    depth: usize,
    /// Its own failure, reported in its parent's run (it could not be opened).
    failure: Option<VfsOmission>,
}

/// The items of the folder whose run is in progress.
struct Run {
    rel: String,
    /// `false` below a folder that is never read: its items are dropped.
    kept: bool,
    entries: Vec<VfsMeta>,
    omitted: Vec<VfsOmission>,
    /// Names of its subfolders.
    dirs: HashSet<String>,
    /// Failures of its subfolders, by name (reported right after their entry).
    child_failures: HashMap<String, VfsOmission>,
    /// The folder itself could not be read (completely).
    failure: Option<VfsOmission>,
    items: u64,
    text: u64,
}

impl Run {
    fn new(rel: String, kept: bool) -> Self {
        Self {
            rel,
            kept,
            entries: Vec::new(),
            omitted: Vec::new(),
            dirs: HashSet::new(),
            child_failures: HashMap::new(),
            failure: None,
            items: 0,
            text: 0,
        }
    }
}

/// Checks a walk's items folder by folder while they arrive.
pub(super) struct Stream<'c, 'a> {
    ctx: &'c WalkContext<'a>,
    tree: Tree,
    expected: HashMap<String, Expected>,
    /// Folders whose content is never read: not entered or failed.
    never_read: HashSet<String>,
    run: Option<Run>,
    received: u64,
}

impl<'c, 'a> Stream<'c, 'a> {
    pub(super) fn new(ctx: &'c WalkContext<'a>) -> Self {
        let root = Expected {
            path: ctx.root.to_string(),
            depth: 0,
            failure: None,
        };
        Self {
            ctx,
            tree: Tree::new(),
            expected: HashMap::from([(String::new(), root)]),
            never_read: HashSet::new(),
            run: None,
            received: 0,
        }
    }

    /// Whether anything arrived (a refusal after that is an error).
    pub(super) fn is_empty(&self) -> bool {
        self.received == 0
    }

    pub(super) fn item(&mut self, item: HashWalkItem) -> io::Result<()> {
        self.received = self.received.saturating_add(1);
        match item {
            HashWalkItem::Entry(entry) if entry.rel.is_empty() => Ok(()),
            HashWalkItem::Entry(entry) => self.entry(entry),
            HashWalkItem::Omitted(omission) if omission.rel.is_empty() => Err(io::Error::other(
                format!("{}: {}", self.ctx.root, omission.detail),
            )),
            HashWalkItem::Omitted(omission) => self.omission(omission),
        }
    }

    fn entry(&mut self, entry: HashWalkEntry) -> io::Result<()> {
        crate::agent_proto::ValidatedRelativePath::parse(&entry.rel)?;
        let meta = entry
            .meta
            .ok_or_else(|| invalid("hash walk entry without its listing attributes"))?;
        let (parent, name) = split(&entry.rel);
        let limits = self.ctx.limits;
        let Some(run) = self.run_of(parent)? else {
            return Ok(());
        };
        count(run, &entry.rel, limits)?;
        let is_dir = entry.is_dir && !meta.is_symlink && !meta.special;
        if is_dir {
            run.dirs.insert(name.to_string());
        }
        run.entries.push(VfsMeta {
            name: name.to_string(),
            is_dir,
            is_symlink: meta.is_symlink,
            special: meta.special,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            btime_ms: meta.btime_ms,
            hidden: meta.hidden,
            system: meta.system,
            id: None,
            // The host's digest stands where a provider lists a free MD5.
            content_md5: entry.digest,
        });
        Ok(())
    }

    fn omission(&mut self, omission: VfsOmission) -> io::Result<()> {
        crate::agent_proto::ValidatedRelativePath::parse(&omission.rel)?;
        let (parent, name) = split(&omission.rel);
        if let Some(run) = self.run.as_mut().filter(|run| run.kept) {
            // The folder in progress could not be read (to its end).
            if run.rel == omission.rel {
                run.failure.get_or_insert(omission);
                return Ok(());
            }
            // A subfolder of it that could not be opened.
            if run.rel == parent && run.dirs.contains(name) {
                let name = name.to_string();
                run.child_failures.entry(name).or_insert(omission);
                return Ok(());
            }
        }
        // A folder that failed before its own run began.
        if let Some(expected) = self.expected.get_mut(&omission.rel) {
            expected.failure.get_or_insert(omission);
            return Ok(());
        }
        if self.never_read.contains(&omission.rel) {
            return Ok(());
        }
        let limits = self.ctx.limits;
        let (parent, name) = (parent.to_string(), name.to_string());
        let Some(run) = self.run_of(&parent)? else {
            return Ok(());
        };
        count(run, &omission.rel, limits)?;
        let VfsOmission { reason, detail, .. } = omission;
        match reason {
            // A link or special file is an entry of its folder's listing,
            // which the protection checks then leave out.
            OmissionReason::Link | OmissionReason::Special => run.entries.push(VfsMeta {
                name,
                is_symlink: reason == OmissionReason::Link,
                special: reason == OmissionReason::Special,
                ..VfsMeta::default()
            }),
            _ => run.omitted.push(VfsOmission {
                rel: name,
                reason,
                detail,
            }),
        }
        Ok(())
    }

    /// The run of `folder`, `None` below a folder that is never read. An
    /// item of another folder ends the run in progress, which is checked.
    fn run_of(&mut self, folder: &str) -> io::Result<Option<&mut Run>> {
        if self.run.as_ref().is_none_or(|run| run.rel != folder) {
            if let Some(done) = self.run.take() {
                self.finish(done)?;
            }
            let kept = if self.expected.contains_key(folder) {
                true
            } else if self.below_never_read(folder) {
                false
            } else {
                return Err(invalid("hash walk sent items outside their folder's run"));
            };
            self.run = Some(Run::new(folder.to_string(), kept));
        }
        Ok(self.run.as_mut().filter(|run| run.kept))
    }

    /// Whether `folder` or a folder above it is never read.
    fn below_never_read(&self, folder: &str) -> bool {
        if self.never_read.is_empty() {
            return false;
        }
        let mut rest = folder;
        loop {
            if self.never_read.contains(rest) {
                return true;
            }
            match rest.rfind('/') {
                Some(index) => rest = &rest[..index],
                None => return false,
            }
        }
    }

    /// Checks one complete folder as the walk over listings checks it.
    fn finish(&mut self, run: Run) -> io::Result<()> {
        if !run.kept {
            return Ok(());
        }
        let ctx = self.ctx;
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        let expected = self
            .expected
            .remove(&run.rel)
            .ok_or_else(|| invalid("hash walk left out a listed folder"))?;
        if expected.depth > MAX_WALK_DEPTH {
            return Err(invalid(&format!(
                "sync tree exceeds {MAX_WALK_DEPTH} levels"
            )));
        }
        let Run {
            rel,
            entries,
            omitted,
            mut dirs,
            mut child_failures,
            failure,
            ..
        } = run;
        if let Some(failure) = expected.failure.or(failure) {
            record_omission(ctx, &rel, failure.reason.into(), true);
            self.never_read.insert(rel);
            return Ok(());
        }
        record_listing_omissions(ctx, &rel, omitted)?;
        let listed = scan_listing(ctx, &expected.path, &rel, entries)?;
        if let Some(observer) = ctx.listed {
            if !ctx.cancel.load(Ordering::Relaxed) {
                observer.listed(&rel, &listed);
            }
        }
        for (path, child) in listed.dirs {
            let name = split(&child).1.to_string();
            dirs.remove(&name);
            let next = Expected {
                path,
                depth: expected.depth + 1,
                failure: child_failures.remove(&name),
            };
            if self.expected.insert(child, next).is_some() {
                return Err(invalid("agent returned duplicate sync paths"));
            }
        }
        // Subfolders `scan_listing` left out: nothing below them is read.
        for name in dirs {
            self.never_read.insert(literal_child(&rel, &name));
        }
        merge_files(&mut self.tree, listed.files);
        Ok(())
    }

    /// The tree once the walk completed: the last folder and every folder
    /// without items (empty or not opened) are checked as well.
    pub(super) fn finish_tree(mut self) -> io::Result<Tree> {
        if let Some(run) = self.run.take() {
            self.finish(run)?;
        }
        let mut rest: Vec<String> = self.expected.keys().cloned().collect();
        rest.sort_by(|left, right| {
            let depth = |rel: &str| rel.matches('/').count();
            depth(left).cmp(&depth(right)).then_with(|| left.cmp(right))
        });
        for rel in rest {
            self.finish(Run::new(rel, true))?;
        }
        if self.ctx.cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        Ok(self.tree)
    }
}

/// Counts one item of the folder in progress: one folder never holds more
/// than the side's walk budget (its check would refuse it anyway).
fn count(run: &mut Run, rel: &str, limits: super::SyncLimits) -> io::Result<()> {
    run.items = run.items.saturating_add(1);
    run.text = run.text.saturating_add(rel.len() as u64);
    if run.items > limits.walk_entries || run.text > limits.walk_text_bytes {
        return Err(invalid("agent walk exceeds its collection budget"));
    }
    Ok(())
}

/// The parent folder and the name of a relative path.
fn split(rel: &str) -> (&str, &str) {
    rel.rsplit_once('/').unwrap_or(("", rel))
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent sync walk canceled")
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "snapshot_host_stream_tests.rs"]
mod tests;
