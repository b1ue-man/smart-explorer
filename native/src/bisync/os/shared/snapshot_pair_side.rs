//! One side's part of the pair's key phases (`snapshot_pair_keys.rs`).
//!
//! A remote side sends its keys only, waits for the plan, sends the runs of
//! the folders the plan names, and replays its whole tree from the records
//! of both sides into the same check as a sent walk (`Stream`), so its
//! snapshot equals the one of the walk without keys. Before it touches the
//! snapshot it makes sure every folder of its tree is held; otherwise it
//! answers `None` and the side takes its walk with known keys instead (a
//! folder changed between the phases, or a fetch failed).
//!
//! A local side reads its tree once more with the walk the agent runs next
//! to its data, in this process, and offers keys and runs: equal folders of
//! the remote side then never cross the connection. Its own snapshot keeps
//! coming from its local walk.
use std::collections::HashMap;
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::folder_key_cache::{FolderCache, Part, Record};
use super::snapshot_dir::WalkContext;
use super::snapshot_host_stream::Stream;
use super::snapshot_keyed::{KeyFiles, KeyedTree};
use super::snapshot_pair_keys::{PairKeys, SideTree};
use crate::agent_proto::{FolderKey, KeyedWireItem};
use crate::vfs::{HashWalkEntry, HashWalkItem, HashWalkMeta, KeyedWalk, KeyedWalkItem};

/// Runs one keyed request of this side and hands every item to `take`;
/// `false` when the side has no such walk (before any item).
pub(super) trait Walker {
    fn walk(
        &self,
        walk: KeyedWalk<'_>,
        take: &mut dyn FnMut(KeyedWalkItem) -> io::Result<()>,
    ) -> io::Result<bool>;
}

/// The remote side `side` through both phases; `None` when it declined or
/// cannot replay its tree (then its walk with known keys runs).
pub(super) fn walk_remote(
    ctx: &WalkContext<'_>,
    (pair, side): (&PairKeys, usize),
    files: &KeyFiles,
    walker: &dyn Walker,
) -> io::Result<Option<KeyedTree>> {
    let mut keys: HashMap<String, FolderKey> = HashMap::new();
    let ran = walker.walk(KeyedWalk::KeysOnly, &mut |item: KeyedWalkItem| match item {
        KeyedWalkItem::Key { rel, key, .. } => match keys.insert(rel, key) {
            None => Ok(()),
            Some(_) => Err(invalid("keyed walk named one folder twice")),
        },
        _ => Err(invalid("keys-only walk sent items")),
    })?;
    let Some(root) = keys.get("").copied().filter(|_| ran) else {
        pair.submit_keys(side, None);
        pair.submit_fetched(side, FolderCache::default());
        return Ok(None);
    };
    pair.submit_keys(
        side,
        Some(SideTree {
            keys: keys.clone(),
            records: Arc::new(FolderCache::default()),
        }),
    );
    let fetch = pair.plan(side, ctx.cancel)?;
    let fetched = match fetch_runs(walker, &fetch, &keys) {
        Ok(fetched) => fetched,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(_) => FolderCache::default(),
    };
    let sent = fetched.len() as u64;
    pair.submit_fetched(side, fetched);
    let sources = pair.sources(ctx.cancel)?;
    let held: Vec<&FolderCache> = sources.iter().map(|cache| &**cache).collect();
    if !FolderCache::holds(&held, root) {
        return Ok(None);
    }
    let mut stream = Stream::new(ctx);
    let folders = FolderCache::replay(&held, root, "", &mut |item: KeyedWalkItem| match item {
        KeyedWalkItem::Run { rel } => stream.run_mark(&rel),
        KeyedWalkItem::Item(item) => stream.item(item),
        KeyedWalkItem::Key { .. } => Ok(()),
    })?;
    let tree = stream.finish_tree()?;
    let mut next = FolderCache::default();
    if next.adopt(root, &held) {
        if let Err(error) = super::folder_key_file::save(&files.own, &next) {
            super::run_log::note(
                "Scan",
                &format!("Ordnerschlüssel nicht gespeichert: {error}"),
            );
        }
    }
    Ok(Some(KeyedTree {
        tree,
        reused: folders.saturating_sub(sent),
        sent,
    }))
}

/// The runs of `folders`, each checked against the key of phase 1; a folder
/// whose run no longer matches is left out (its folder changed).
fn fetch_runs(
    walker: &dyn Walker,
    folders: &[String],
    keys: &HashMap<String, FolderKey>,
) -> io::Result<FolderCache> {
    let mut runs: HashMap<String, Vec<Part>> = HashMap::new();
    if !folders.is_empty() {
        let mut current: Option<String> = None;
        let ran = walker.walk(
            KeyedWalk::Runs { folders },
            &mut |item: KeyedWalkItem| match item {
                KeyedWalkItem::Run { rel } => {
                    runs.insert(rel.clone(), Vec::new());
                    current = Some(rel);
                    Ok(())
                }
                KeyedWalkItem::Item(item) => {
                    let folder = current
                        .as_deref()
                        .ok_or_else(|| invalid("run item before its mark"))?;
                    let part = Part::of(folder, &item)?;
                    if let Some(run) = runs.get_mut(folder) {
                        run.push(part);
                    }
                    Ok(())
                }
                KeyedWalkItem::Key { .. } => Err(invalid("run listing sent a key")),
            },
        )?;
        if !ran {
            return Err(invalid("run listing refused"));
        }
    }
    Ok(records(runs, keys))
}

/// Records of runs whose key, with the subfolders' keys of `keys`, is the
/// key `keys` names for their folder. A subfolder without a key was not
/// walked (the walk's filter left it out); a walked one whose key is missing
/// makes the run's key differ, so its record is left out.
fn records(runs: HashMap<String, Vec<Part>>, keys: &HashMap<String, FolderKey>) -> FolderCache {
    let mut records = FolderCache::default();
    for (rel, parts) in runs {
        let mut record = Record {
            parts,
            children: Vec::new(),
        };
        record.children = record.walked_children(&rel, keys);
        match keys.get(&rel) {
            Some(key) if record.key() == *key => records.insert(*key, record),
            _ => {}
        }
    }
    records
}

/// The local side `side`: its keys and runs from the agent's walk in this
/// process, with the job's filter as the remote side's walk applies it;
/// declines when that walk fails.
pub(super) fn provide_local(
    (pair, side): (&PairKeys, usize),
    root: &str,
    (md5, filter): (bool, Option<Arc<crate::agent_proto::WalkFilterSpec>>),
    cancel: &AtomicBool,
) {
    let algorithm = if md5 {
        crate::agent_proto::digest::MD5
    } else {
        crate::agent_proto::digest::NONE
    };
    let reads = crate::agent_proto::LocalReads {
        // The checksum compare mode reads every file; the snapshot walk of
        // the same side takes the digests of this run.
        fresh: md5,
        run: pair.run_digests(side),
    };
    // A filter the walk cannot apply is left to the snapshot's own check.
    let filter = filter.and_then(|spec| spec.compile().ok());
    let tree =
        crate::agent_proto::local_keyed_reading(root, algorithm, (reads, filter.as_ref()), cancel)
            .ok()
            .and_then(|collected| {
                let keys: HashMap<String, FolderKey> = collected.keys.into_iter().collect();
                let mut runs = HashMap::new();
                for (rel, items) in collected.runs {
                    let parts = items
                        .into_iter()
                        .map(|item| Part::of(&rel, &from_wire(item)))
                        .collect::<io::Result<Vec<Part>>>()
                        .ok()?;
                    runs.insert(rel, parts);
                }
                let records = records(runs, &keys);
                Some(SideTree {
                    keys,
                    records: Arc::new(records),
                })
            });
    pair.submit_keys(side, tree);
    pair.submit_fetched(side, FolderCache::default());
}

fn from_wire(item: KeyedWireItem) -> HashWalkItem {
    match item {
        KeyedWireItem::Entry(entry) => HashWalkItem::Entry(HashWalkEntry {
            rel: entry.rel,
            is_dir: entry.is_dir,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            digest: entry.digest,
            meta: entry.meta.map(|meta| HashWalkMeta {
                hidden: meta.hidden,
                system: meta.system,
                btime_ms: meta.btime_ms,
                is_symlink: meta.is_symlink,
                special: meta.special,
            }),
        }),
        KeyedWireItem::Omitted(omission) => {
            HashWalkItem::Omitted(crate::agent::ext_wire::omission_from_wire(omission))
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
