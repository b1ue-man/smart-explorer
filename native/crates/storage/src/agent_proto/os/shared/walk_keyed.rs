//! `WalkKeyed` (`walk-keys-v1`) on the agent's own filesystem: the walk of
//! `WalkHashed3` with `meta` whose folder runs go through a `KeyTree`
//! (`folder_key.rs`): every folder's run as `HashPart`s, then its `FolderKey`
//! end mark once its subtree is sent, or one `same` mark instead of a
//! subtree whose key the client named as known. The items and their order
//! inside a run are those of `WalkHashed3`. With `keys_only` the walk sends
//! only every folder's key; `ListRuns` sends the runs of named folders only
//! (the two phases of a pair comparison, `bisync::snapshot_pair_keys`).
//! `local_keyed` runs the same walk in this process and collects runs and
//! keys instead of sending them; `Collected::below` does it for one folder
//! a local sync side's own walk does not enter (`bisync::snapshot_local_keys`).
use std::collections::HashSet;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use super::folder_key::{
    entry_part, parse_known, EntryKind, FolderKey, KeyEmit, KeyTree, Mode, RunKeys, Shape,
};
use super::key_tally::KeyTally;
use super::ops_types::digest;
use super::session::{emit, Sink};
use super::{Frame, HostFilter, LocalReads, WalkScope, WireHashEntry, WireOmission};

/// Estimated encoded bytes after which a `HashPart` of a run is sent.
const PART_BYTES: usize = 256 * 1024;

/// One item of a keyed run.
pub enum WireItem {
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

/// The runs and keys a walk in this process decided (`local_keyed`).
#[derive(Default)]
pub struct Collected {
    pub runs: Vec<(String, Vec<WireItem>)>,
    pub keys: Vec<(String, FolderKey)>,
    /// What each keyed folder's subtree holds (`Mode::KeysOnly` walks).
    pub tallies: Vec<(String, KeyTally)>,
}

impl KeyEmit<WireItem> for Collected {
    fn run(&mut self, rel: &str, items: Vec<WireItem>) -> io::Result<()> {
        self.runs.push((rel.to_string(), items));
        Ok(())
    }

    fn key(
        &mut self,
        rel: &str,
        key: &FolderKey,
        _same: bool,
        tally: Option<KeyTally>,
    ) -> io::Result<()> {
        self.keys.push((rel.to_string(), *key));
        if let Some(tally) = tally {
            self.tallies.push((rel.to_string(), tally));
        }
        Ok(())
    }
}

/// Where the runs of a keyed walk go: the walk's key tree, and a collector
/// when the walk runs in this process.
#[derive(Clone, Copy)]
pub(super) struct KeyedSetup<'a> {
    pub(super) tree: &'a KeyTree<WireItem>,
    pub(super) collect: Option<&'a Mutex<Collected>>,
    /// How a walk in this process reads file contents (`local_keyed`).
    pub(super) reads: LocalReads<'a>,
    /// The job's filter (`walk_filter.rs`).
    pub(super) filter: Option<&'a HostFilter>,
}

/// How a walk reads file contents and which filter it applies.
#[derive(Clone, Copy, Default)]
pub(super) struct Scope<'a> {
    pub(super) reads: LocalReads<'a>,
    pub(super) filter: Option<&'a HostFilter>,
}

/// The compiled filter of a request's `scope`; `None` when the walk cannot
/// apply it (answered as an unsupported extension, before any item).
pub(super) fn request_filter(scope: &WalkScope) -> Option<Option<HostFilter>> {
    super::walk_filter::compiled(scope).ok()
}

/// The keyed state of one walking thread: the run of its current folder.
pub(super) struct Keyed<'a> {
    setup: KeyedSetup<'a>,
    run: Option<RunKeys<WireItem>>,
}

impl<'a> Keyed<'a> {
    pub(super) fn new(setup: KeyedSetup<'a>) -> Self {
        Self { setup, run: None }
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
        self.entry_walked(entry, true)
    }

    /// A folder entry whose content the walk leaves out (its filter): the
    /// folder's key covers it without a key of its own (`RunKeys::skip_dir`).
    pub(super) fn skipped_dir(&mut self, entry: WireHashEntry) -> io::Result<()> {
        self.entry_walked(entry, false)
    }

    fn entry_walked(&mut self, entry: WireHashEntry, walked: bool) -> io::Result<()> {
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
        let shape = Shape {
            name: &name,
            kind,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
        };
        let item = WireItem::Entry(entry);
        let cost = item.cost();
        run.entry(shape, part, item, cost);
        if !walked {
            run.skip_dir(&name);
        }
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
        match self.setup.collect {
            Some(collect) => {
                let mut collected = collect.lock().unwrap_or_else(|p| p.into_inner());
                self.setup.tree.finish_run(run, &mut *collected)
            }
            None => self.setup.tree.finish_run(run, &mut Parts { sink, id }),
        }
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

    fn key(
        &mut self,
        rel: &str,
        key: &FolderKey,
        same: bool,
        tally: Option<KeyTally>,
    ) -> io::Result<()> {
        emit(
            self.sink,
            self.id,
            &Frame::FolderKey {
                rel: rel.to_string(),
                key: *key,
                same,
                tally,
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

/// `WalkKeyed` of `root` (`digest::NONE` or `digest::MD5`) as `scope` reads
/// and filters it, then `End`.
pub fn handle_walk_keyed(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, min_bytes, known, keys_only): (u8, u64, &[u8], bool),
    scope: &WalkScope,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let filter = request_filter(scope);
    if !matches!(algorithm, digest::NONE | digest::MD5) || filter.is_none() {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    let mode = if keys_only {
        Mode::KeysOnly
    } else {
        Mode::Marks
    };
    let tree = KeyTree::with_mode("", parse_known(known)?, mode);
    let setup = KeyedSetup {
        tree: &tree,
        collect: None,
        reads: LocalReads {
            fresh: scope.fresh,
            run: None,
        },
        filter: filter.as_ref().and_then(Option::as_ref),
    };
    let walk = (algorithm, min_bytes, None, true);
    super::walk_hashed::walk_keyed(sink, id, root, walk, setup, cancel)?;
    if !tree.complete() {
        return Err(invalid("keyed walk ended before every folder was complete"));
    }
    emit(sink, id, &Frame::End)
}

/// `ListRuns`: the runs of `folders` below `root`, each without its
/// subfolders, then `End`.
pub fn handle_list_runs(
    sink: &Sink,
    id: u64,
    root: &str,
    (algorithm, folders): (u8, &[String]),
    scope: &WalkScope,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let filter = request_filter(scope);
    if !matches!(algorithm, digest::NONE | digest::MD5) || filter.is_none() {
        return emit(
            sink,
            id,
            &Frame::Err(super::UNSUPPORTED_EXTENSION.to_string()),
        );
    }
    let mut unique = HashSet::new();
    let folders: Vec<String> = folders
        .iter()
        .filter(|rel| unique.insert(rel.as_str()))
        .cloned()
        .collect();
    let tree = KeyTree::with_mode("", HashSet::new(), Mode::RunsOnly);
    let setup = KeyedSetup {
        tree: &tree,
        collect: None,
        reads: LocalReads {
            fresh: scope.fresh,
            run: None,
        },
        filter: filter.as_ref().and_then(Option::as_ref),
    };
    let walk = (algorithm, 0, Some(folders.as_slice()), false);
    super::walk_hashed::walk_keyed(sink, id, root, walk, setup, cancel)?;
    emit(sink, id, &Frame::End)
}

/// The runs and keys of the local tree `root`, read by the walk the agent
/// runs next to its data, in this process (the keys a walk next to the data
/// forms for the same tree).
pub fn local_keyed(root: &str, algorithm: u8, cancel: &AtomicBool) -> io::Result<Collected> {
    local_keyed_reading(root, algorithm, (LocalReads::default(), None), cancel)
}

/// `local_keyed` with the way file contents are read (a fresh read: the
/// checksum compare mode; the digests of a run, `RunDigests`) and with the
/// job's filter, as the walk of the remote side applies it (equal filtered
/// folders get equal keys on both sides).
pub fn local_keyed_reading(
    root: &str,
    algorithm: u8,
    (reads, filter): (LocalReads<'_>, Option<&HostFilter>),
    cancel: &AtomicBool,
) -> io::Result<Collected> {
    let scope = Scope { reads, filter };
    collect_local((root, ""), algorithm, Mode::Marks, scope, cancel)
}

impl Collected {
    /// Only the keys of the local tree `root` (the walk of `local_keyed`
    /// without keeping any run), each with what its subtree holds: the
    /// folder `""` is the root, every other folder is named by its path
    /// below it. For callers that compare keys and never replay a run
    /// (merge copies, `transfer::update`).
    pub fn keys_of(
        root: &str,
        algorithm: u8,
        cancel: &AtomicBool,
    ) -> io::Result<Vec<(String, FolderKey, KeyTally)>> {
        let scope = Scope::default();
        let collected = collect_local((root, ""), algorithm, Mode::KeysOnly, scope, cancel)?;
        let mut tallies: std::collections::HashMap<String, KeyTally> =
            collected.tallies.into_iter().collect();
        collected
            .keys
            .into_iter()
            .map(|(rel, key)| {
                let tally = tallies
                    .remove(&rel)
                    .ok_or_else(|| invalid("keys-only walk gave a folder no tally"))?;
                Ok((rel, key, tally))
            })
            .collect()
    }

    /// `local_keyed_reading` of the folder `rel` below the local tree
    /// `root` and everything below it: the runs and keys of that subtree,
    /// named by their paths below `root`, so the filter sees the paths it
    /// sees in a walk of the whole tree. For a folder a local side's own
    /// walk does not enter although the walk next to the data on the other
    /// side does (`bisync::snapshot_local_keys`).
    pub fn below(
        (root, rel): (&str, &str),
        algorithm: u8,
        (reads, filter): (LocalReads<'_>, Option<&HostFilter>),
        cancel: &AtomicBool,
    ) -> io::Result<Collected> {
        let scope = Scope { reads, filter };
        collect_local((root, rel), algorithm, Mode::Marks, scope, cancel)
    }
}

/// The keyed walk of the folder `rel` below `root` (`""`: the whole tree)
/// with everything below it, collected in this process.
fn collect_local(
    (root, rel): (&str, &str),
    algorithm: u8,
    mode: Mode,
    scope: Scope<'_>,
    cancel: &AtomicBool,
) -> io::Result<Collected> {
    let tree = KeyTree::with_mode(rel, HashSet::new(), mode);
    let collect = Mutex::new(Collected::default());
    let setup = KeyedSetup {
        tree: &tree,
        collect: Some(&collect),
        reads: scope.reads,
        filter: scope.filter,
    };
    let sink: Sink = std::sync::Arc::new(Mutex::new(
        Box::new(io::sink()) as Box<dyn std::io::Write + Send>
    ));
    let folders = [rel.to_string()];
    let only = (!rel.is_empty()).then_some(&folders[..]);
    let walk = (algorithm, 0, only, true);
    super::walk_hashed::walk_keyed(&sink, 0, root, walk, setup, cancel)?;
    if !tree.complete() {
        return Err(invalid(
            "local keyed walk ended before every folder was complete",
        ));
    }
    Ok(collect.into_inner().unwrap_or_else(|p| p.into_inner()))
}

/// `hidden` of an entry in the walk of a local sync side: on Windows the
/// file attribute (as the Share host lists it, `vfs::local_attrs`), where a
/// dot name is an ordinary name; elsewhere a leading dot, as the agent and
/// SFTP list it.
#[cfg(windows)]
pub(super) fn local_hidden(_name: &str, metadata: &std::fs::Metadata) -> bool {
    super::local_platform::hidden_attribute(metadata)
}

/// See the Windows variant.
#[cfg(not(windows))]
pub(super) fn local_hidden(name: &str, _metadata: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

#[cfg(test)]
#[path = "local_keyed_hidden_tests.rs"]
mod hidden_tests;
