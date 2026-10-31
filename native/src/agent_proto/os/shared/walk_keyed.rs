//! `WalkKeyed` (`walk-keys-v1`) on the agent's own filesystem: the walk of
//! `WalkHashed3` with `meta` whose folder runs go through a `KeyTree`
//! (`folder_key.rs`): every folder's run as `HashPart`s, then its `FolderKey`
//! end mark once its subtree is sent, or one `same` mark instead of a
//! subtree whose key the client named as known. The items and their order
//! inside a run are those of `WalkHashed3`.
use std::io;
use std::sync::atomic::AtomicBool;

use super::folder_key::{entry_part, parse_known, EntryKind, FolderKey, KeyEmit, KeyTree, RunKeys};
use super::ops_types::digest;
use super::session::{emit, Sink};
use super::{Frame, WireHashEntry, WireOmission};

/// Estimated encoded bytes after which a `HashPart` of a run is sent.
const PART_BYTES: usize = 256 * 1024;

/// One item of a keyed run.
pub(super) enum WireItem {
    Entry(WireHashEntry),
    Omitted(WireOmission),
}

impl WireItem {
    fn cost(&self) -> usize {
        match self {
            WireItem::Entry(entry) => {
                entry.rel.len() + entry.digest.as_ref().map_or(0, String::len) + 48
            }
            WireItem::Omitted(omission) => omission.rel.len() + omission.detail.len() + 16,
        }
    }
}

/// The keyed state of one walking thread: the run of its current folder.
pub(super) struct Keyed<'a> {
    tree: &'a KeyTree<WireItem>,
    run: Option<RunKeys<WireItem>>,
}

impl<'a> Keyed<'a> {
    pub(super) fn new(tree: &'a KeyTree<WireItem>) -> Self {
        Self { tree, run: None }
    }

    /// The folder `rel` starts.
    pub(super) fn begin(&mut self, rel: &str) -> io::Result<()> {
        if self.run.is_some() {
            return Err(invalid("keyed walk began a folder inside another"));
        }
        self.run = Some(RunKeys::new(rel));
        Ok(())
    }

    fn current(&mut self) -> io::Result<&mut RunKeys<WireItem>> {
        self.run
            .as_mut()
            .ok_or_else(|| invalid("keyed walk item outside a folder"))
    }

    pub(super) fn entry(&mut self, entry: WireHashEntry) -> io::Result<()> {
        let run = self.current()?;
        let name = run
            .name_of(&entry.rel)?
            .ok_or_else(|| invalid("keyed walk entry names its own folder"))?
            .to_string();
        let meta = entry.meta.unwrap_or_default();
        let kind = EntryKind::of(entry.is_dir, meta.is_symlink, meta.special);
        let part = entry_part(
            &name,
            kind,
            entry.size,
            entry.mtime_ms,
            entry.digest.as_deref(),
            meta.hidden,
        );
        let item = WireItem::Entry(entry);
        let cost = item.cost();
        run.entry(&name, kind, part, item, cost);
        Ok(())
    }

    pub(super) fn omit(&mut self, omission: WireOmission) -> io::Result<()> {
        let run = self.current()?;
        let reason = omission.reason;
        let name = run.name_of(&omission.rel)?.map(str::to_string);
        let item = WireItem::Omitted(omission);
        let cost = item.cost();
        match name {
            Some(name) => run.omission(&name, reason, item, cost),
            None => run.failure(reason, item, cost),
        }
        Ok(())
    }

    /// The current folder's run ended.
    pub(super) fn end(&mut self, sink: &Sink, id: u64) -> io::Result<()> {
        let run = self
            .run
            .take()
            .ok_or_else(|| invalid("keyed walk ended a folder it never began"))?;
        self.tree.finish_run(run, &mut Parts { sink, id })
    }
}

/// Sends what a `KeyTree` decided.
struct Parts<'s> {
    sink: &'s Sink,
    id: u64,
}

impl KeyEmit<WireItem> for Parts<'_> {
    fn run(&mut self, rel: &str, items: Vec<WireItem>) -> io::Result<()> {
        emit(self.sink, self.id, &Frame::FolderRun(rel.to_string()))?;
        let (mut entries, mut omitted, mut bytes) = (Vec::new(), Vec::new(), 0usize);
        for item in items {
            // The client forwards a part's entries before its omissions.
            let flush =
                bytes >= PART_BYTES || matches!(item, WireItem::Entry(_)) && !omitted.is_empty();
            if flush {
                self.part(&mut entries, &mut omitted)?;
                bytes = 0;
            }
            bytes += item.cost();
            match item {
                WireItem::Entry(entry) => entries.push(entry),
                WireItem::Omitted(omission) => omitted.push(omission),
            }
        }
        self.part(&mut entries, &mut omitted)
    }

    fn key(&mut self, rel: &str, key: &FolderKey, same: bool) -> io::Result<()> {
        emit(
            self.sink,
            self.id,
            &Frame::FolderKey {
                rel: rel.to_string(),
                key: *key,
                same,
            },
        )
    }
}

impl Parts<'_> {
    fn part(
        &mut self,
        entries: &mut Vec<WireHashEntry>,
        omitted: &mut Vec<WireOmission>,
    ) -> io::Result<()> {
        if entries.is_empty() && omitted.is_empty() {
            return Ok(());
        }
        emit(
            self.sink,
            self.id,
            &Frame::HashPart {
                entries: std::mem::take(entries),
                omitted: std::mem::take(omitted),
            },
        )
    }
}

/// `WalkKeyed` of `root` (`digest::NONE` or `digest::MD5`), then `End`.
pub(crate) fn handle_walk_keyed(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, min_bytes, known): (u8, u64, &[u8]),
    cancel: &AtomicBool,
) -> io::Result<()> {
    if !matches!(algorithm, digest::NONE | digest::MD5) {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    let tree = KeyTree::new("", parse_known(known)?);
    super::walk_hashed::walk_keyed(sink, id, root, (algorithm, min_bytes), &tree, cancel)?;
    if !tree.complete() {
        return Err(invalid("keyed walk ended before every folder was complete"));
    }
    emit(sink, id, &Frame::End)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
