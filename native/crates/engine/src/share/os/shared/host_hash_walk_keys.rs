//! Folder keys of a host hash walk (`hash_walk_keys_v1`,
//! `agent_proto::folder_key`): with `keys` every folder's run goes through
//! the walk's `KeyTree` instead of straight into the stream. A run is sent
//! whole (in portions of `LIST_BATCH_MAX_BYTES`, never an entry after an
//! omission in one portion), its folder's `Key` follows once everything
//! below it is sent, and a folder whose key the client named as known is
//! one `Key { same }` instead of its subtree. The totals of `Done` count
//! what was sent.
use std::io;
use std::sync::atomic::Ordering;

use super::{Batch, Shared};
use crate::agent_proto::{
    entry_part, EntryKind, FolderKey, KeyEmit, KeyMode, KeyShape, KeyTree, RunKeys,
};
use crate::share::{
    fs_response::{FsHashEntry, FsHashWalkMessage, FsOmission, LIST_BATCH_MAX_BYTES},
    host_stream,
    wire::FsResponse,
};
use crate::vfs::{KeyTally, OmissionReason};

/// One item of a keyed run.
pub(super) type Item = Result<FsHashEntry, FsOmission>;

/// The key tree of a walk whose request asked for keys.
pub(super) fn tree(request: &crate::share::wire::FsHashWalk) -> io::Result<Option<KeyTree<Item>>> {
    if !request.keys {
        return Ok(None);
    }
    if !request.meta {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Ordnerschlüssel nur mit Eintragsmerkmalen",
        ));
    }
    let known = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        request.known.as_bytes(),
    )
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bekannte Schlüssel ungültig"))?;
    let mode = if !request.folders.is_empty() {
        KeyMode::RunsOnly
    } else if request.keys_only {
        KeyMode::KeysOnly
    } else {
        KeyMode::Marks
    };
    let known = crate::agent_proto::parse_known(&known)?;
    Ok(Some(KeyTree::with_mode("", known, mode)))
}

fn cost(item: &Item) -> usize {
    match item {
        Ok(entry) => entry.wire_bytes(),
        Err(hole) => hole.wire_bytes(),
    }
    .saturating_mul(6)
}

/// The folder `rel` starts on this thread.
pub(super) fn begin(batch: &mut Batch<'_, '_>, rel: &str) -> io::Result<()> {
    if batch.run.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Ordner begann innerhalb eines anderen",
        ));
    }
    batch.run = Some(RunKeys::new(rel));
    Ok(())
}

/// One item of the current folder.
pub(super) fn add(batch: &mut Batch<'_, '_>, item: Item) -> io::Result<()> {
    let item_cost = cost(&item);
    if item_cost > LIST_BATCH_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Prüfsummenpfad überschreitet das Drahtformat",
        ));
    }
    let run = batch.run.as_mut().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Eintrag außerhalb eines Ordners",
        )
    })?;
    match &item {
        Ok(entry) => {
            let name = run
                .name_of(&entry.rel)?
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Eintrag nennt seinen Ordner")
                })?
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
            let shape = KeyShape {
                name: &name,
                kind,
                size: entry.size,
                mtime_ms: entry.mtime_ms,
            };
            run.entry(shape, part, item, item_cost);
        }
        Err(hole) => {
            let reason = OmissionReason::from(hole.reason).code();
            match run.name_of(&hole.rel)?.map(str::to_string) {
                Some(name) => run.omission(&name, reason, item, item_cost),
                None => run.failure(reason, item, item_cost),
            }
        }
    }
    Ok(())
}

/// The current folder's run ended.
pub(super) fn end(batch: &mut Batch<'_, '_>) -> io::Result<()> {
    let shared = batch.shared;
    let (Some(tree), Some(run)) = (shared.keys.as_ref(), batch.run.take()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Ordnerende ohne Ordner",
        ));
    };
    tree.finish_run(run, &mut Stream { shared })
}

struct Stream<'s, 'a> {
    shared: &'s Shared<'a>,
}

impl Stream<'_, '_> {
    fn portion(
        &self,
        entries: &mut Vec<FsHashEntry>,
        omitted: &mut Vec<FsOmission>,
    ) -> io::Result<()> {
        if entries.is_empty() && omitted.is_empty() {
            return Ok(());
        }
        let shared = self.shared;
        shared
            .entries
            .fetch_add(entries.len() as u64, Ordering::Relaxed);
        shared
            .omitted
            .fetch_add(omitted.len() as u64, Ordering::Relaxed);
        host_stream::emit(
            shared.tx,
            shared.cancel,
            FsResponse::HashWalk {
                message: FsHashWalkMessage::Batch {
                    entries: std::mem::take(entries),
                    omitted: std::mem::take(omitted),
                },
            },
        )
    }
}

impl KeyEmit<Item> for Stream<'_, '_> {
    fn run(&mut self, rel: &str, items: Vec<Item>) -> io::Result<()> {
        host_stream::emit(
            self.shared.tx,
            self.shared.cancel,
            FsResponse::HashWalk {
                message: FsHashWalkMessage::Run {
                    rel: rel.to_string(),
                },
            },
        )?;
        let (mut entries, mut omitted, mut bytes) = (Vec::new(), Vec::new(), 0usize);
        for item in items {
            let item_cost = cost(&item);
            if bytes + item_cost > LIST_BATCH_MAX_BYTES || item.is_ok() && !omitted.is_empty() {
                self.portion(&mut entries, &mut omitted)?;
                bytes = 0;
            }
            bytes += item_cost;
            match item {
                Ok(entry) => entries.push(entry),
                Err(hole) => omitted.push(hole),
            }
        }
        self.portion(&mut entries, &mut omitted)
    }

    fn key(
        &mut self,
        rel: &str,
        key: &FolderKey,
        same: bool,
        tally: Option<KeyTally>,
    ) -> io::Result<()> {
        host_stream::emit(
            self.shared.tx,
            self.shared.cancel,
            FsResponse::HashWalk {
                message: FsHashWalkMessage::Key {
                    rel: rel.to_string(),
                    key: key.iter().map(|byte| format!("{byte:02x}")).collect(),
                    same,
                    tally,
                },
            },
        )
    }
}
