//! One snapshot side from a walk next to the data (`snapshot_agent.rs`).
//! The streamed entries are grouped into the listings of their folders and
//! then checked folder by folder with `scan_listing`, exactly as the walk
//! over separate listings checks them (filters, hidden entries, protected
//! names, links, special files, budget, duplicate names, early copies), so
//! the snapshot, its folders, filtered files, omissions and duplicates are
//! those of that walk. Only folders that walk would read are kept: entries
//! below a folder the filters or the own-name rules leave out are dropped
//! as they arrive.
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::atomic::Ordering;

use super::snapshot_dir::{record_listing_omissions, record_omission, scan_listing, WalkContext};
use super::snapshot_walk::{merge_files, MAX_WALK_DEPTH};
use super::types::Tree;
use crate::vfs::{HashWalkEntry, HashWalkItem, OmissionReason, VfsMeta, VfsOmission};

/// The streamed listing of one folder.
#[derive(Default)]
struct Folder {
    entries: Vec<VfsMeta>,
    omitted: Vec<VfsOmission>,
    /// The folder itself could not be read (the walk reported it, not a child).
    failure: Option<VfsOmission>,
}

/// Groups a host walk's items into folder listings.
pub(super) struct Collector<'c, 'a> {
    ctx: &'c WalkContext<'a>,
    folders: HashMap<String, Folder>,
    /// Folders the walk over listings never enters.
    dropped: HashSet<String>,
    root_failure: Option<VfsOmission>,
    nodes: u64,
    text: u64,
}

impl<'c, 'a> Collector<'c, 'a> {
    pub(super) fn new(ctx: &'c WalkContext<'a>) -> Self {
        Self {
            ctx,
            folders: HashMap::from([(String::new(), Folder::default())]),
            dropped: HashSet::new(),
            root_failure: None,
            nodes: 0,
            text: 0,
        }
    }

    /// Whether anything arrived (a refusal after that is an error).
    pub(super) fn is_empty(&self) -> bool {
        self.nodes == 0 && self.root_failure.is_none()
    }

    pub(super) fn item(&mut self, item: HashWalkItem) -> io::Result<()> {
        match item {
            HashWalkItem::Entry(entry) if entry.rel.is_empty() => Ok(()),
            HashWalkItem::Entry(entry) => self.entry(entry),
            HashWalkItem::Omitted(omission) if omission.rel.is_empty() => {
                self.root_failure.get_or_insert(omission);
                Ok(())
            }
            HashWalkItem::Omitted(omission) => self.omission(omission),
        }
    }

    /// The parent folder and the name of `rel`, `None` below a dropped
    /// folder; counts the item against the walk's collection budget.
    fn parent(&mut self, rel: &str) -> io::Result<Option<(String, String)>> {
        crate::agent_proto::ValidatedRelativePath::parse(rel)?;
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        if self.dropped.contains(parent) {
            return Ok(None);
        }
        if !self.folders.contains_key(parent) {
            return Err(invalid("hash walk entry arrived before its folder"));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text = self.text.saturating_add(rel.len() as u64);
        if self.nodes > self.ctx.limits.walk_entries || self.text > self.ctx.limits.walk_text_bytes
        {
            return Err(invalid("agent walk exceeds its collection budget"));
        }
        Ok(Some((parent.to_string(), name.to_string())))
    }

    fn folder(&mut self, parent: &str) -> io::Result<&mut Folder> {
        self.folders
            .get_mut(parent)
            .ok_or_else(|| invalid("hash walk entry arrived before its folder"))
    }

    fn entry(&mut self, entry: HashWalkEntry) -> io::Result<()> {
        let meta = entry.meta.unwrap_or_default();
        let is_dir = entry.is_dir && !meta.is_symlink && !meta.special;
        let rel = entry.rel;
        let Some((parent, name)) = self.parent(&rel)? else {
            return Ok(());
        };
        let listed = VfsMeta {
            name,
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
        };
        let enters = is_dir && !self.never_entered(&rel, &listed);
        self.folder(&parent)?.entries.push(listed);
        if !is_dir {
            return Ok(());
        }
        if enters {
            if self.folders.insert(rel, Folder::default()).is_some() {
                return Err(invalid("agent returned duplicate sync paths"));
            }
        } else {
            self.dropped.insert(rel);
        }
        Ok(())
    }

    /// Folders `scan_listing` never queues, whatever else it finds: left out
    /// by the filters or hidden, or an engine, staging or trash name.
    fn never_entered(&self, rel: &str, listed: &VfsMeta) -> bool {
        (!self.ctx.filter.include_hidden && listed.hidden)
            || self.ctx.filter.ignored(rel, true)
            || super::paths::is_engine_name(&listed.name)
            || crate::vfs::is_staging_name(&listed.name)
            || crate::apptrash::excluded_name(&listed.name)
    }

    fn omission(&mut self, omission: VfsOmission) -> io::Result<()> {
        if self.dropped.contains(&omission.rel) {
            return Ok(());
        }
        if let Some(folder) = self.folders.get_mut(&omission.rel) {
            // A listed folder that could not be read itself.
            folder.failure.get_or_insert(omission);
            return Ok(());
        }
        let VfsOmission {
            rel,
            reason,
            detail,
        } = omission;
        let Some((parent, name)) = self.parent(&rel)? else {
            return Ok(());
        };
        let folder = self.folder(&parent)?;
        match reason {
            // A link or special file is an entry of its folder's listing,
            // which the protection checks then leave out.
            OmissionReason::Link | OmissionReason::Special => folder.entries.push(VfsMeta {
                name,
                is_symlink: reason == OmissionReason::Link,
                special: reason == OmissionReason::Special,
                ..VfsMeta::default()
            }),
            _ => folder.omitted.push(VfsOmission {
                rel: name,
                reason,
                detail,
            }),
        }
        Ok(())
    }

    /// Checks the collected folders as the walk over listings reads them and
    /// returns the tree; the context collects everything else.
    pub(super) fn replay(mut self) -> io::Result<Tree> {
        let ctx = self.ctx;
        if let Some(failure) = self.root_failure.take() {
            return Err(io::Error::other(format!(
                "{}: {}",
                ctx.root, failure.detail
            )));
        }
        let mut tree = Tree::new();
        let mut queue = vec![(ctx.root.to_string(), String::new(), 0usize)];
        while let Some((dir, rel, depth)) = queue.pop() {
            if ctx.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            if depth > MAX_WALK_DEPTH {
                return Err(invalid(&format!(
                    "sync tree exceeds {MAX_WALK_DEPTH} levels"
                )));
            }
            let folder = self
                .folders
                .remove(&rel)
                .ok_or_else(|| invalid("hash walk left out a listed folder"))?;
            if let Some(failure) = folder.failure {
                record_omission(ctx, &rel, failure.reason.into(), true);
                continue;
            }
            record_listing_omissions(ctx, &rel, folder.omitted)?;
            let listed = scan_listing(ctx, &dir, &rel, folder.entries)?;
            if let Some(observer) = ctx.listed {
                if !ctx.cancel.load(Ordering::Relaxed) {
                    observer.listed(&rel, &listed);
                }
            }
            merge_files(&mut tree, listed.files);
            queue.extend(
                listed
                    .dirs
                    .into_iter()
                    .map(|(path, rel)| (path, rel, depth + 1)),
            );
        }
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err(canceled());
        }
        Ok(tree)
    }
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "agent sync walk canceled")
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
