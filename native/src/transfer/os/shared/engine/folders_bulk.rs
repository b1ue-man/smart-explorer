//! Folders created in bulk where the remote target offers it
//! (`vfs::create_dirs`, Share `create_dirs_v1`): a new nested path costs one
//! request for all its missing levels instead of one per level, and
//! `ensure_many` creates the folders of a whole packet or listing at once.
//! Every folder keeps its slot in the register, so concurrent askers wait
//! for it exactly as for a single creation. Nested folders keep their merge
//! semantics (an existing plain folder is merged into); new top-level
//! folders keep their exclusive creation with keep-both numbering and are
//! never part of a bulk request. The host reports which folders it created,
//! so "this register created it or one of its parents" stays exact.
use super::super::super::engine_names::parent_rel;
use super::super::view::Side;
use super::{lock, FolderError, FolderRegister, Slot};
use crate::vfs::Backend;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Instant;

/// What one folder of a bulk creation ended with.
type Settled = HashMap<String, Result<bool, FolderError>>;

impl FolderRegister<'_> {
    /// `rel` (whose slot the caller holds as `Creating`) together with its
    /// missing parents in one request; `None` where bulk creation does not
    /// apply (local target, a planned top-level folder, no support).
    pub(super) fn create_bulk(
        &self,
        rel: &str,
        cancel: &AtomicBool,
    ) -> Option<Result<bool, FolderError>> {
        let backend = self.bulk_backend()?;
        if lock(&self.exclusive).contains_key(rel) {
            return None;
        }
        let mut mine = HashSet::new();
        let mut claimed = self.claim_chain(&self.ancestors(rel), &mut mine);
        claimed.push(rel.to_string());
        let mut settled = self.bulk(backend, claimed, cancel);
        let own = settled.remove(rel);
        for (folder, result) in &settled {
            self.settle_slot(folder, result);
        }
        self.changed.notify_all();
        Some(own.unwrap_or_else(|| Err(lost(rel))))
    }

    /// Creates the folders `rels` and their missing parents in as few
    /// requests as the target allows (for example the folders of a packet
    /// or of a listing, before their files ask for them). Folders already
    /// known to the register are left alone; a failure stays in its folder's
    /// slot for the files that ask for it. Without bulk support this does
    /// nothing: each folder is still created when its first file asks.
    pub(crate) fn ensure_many(&self, rels: &[&str], cancel: &AtomicBool) {
        let Some(backend) = self.bulk_backend() else {
            return;
        };
        let mut mine = HashSet::new();
        let mut claimed = Vec::new();
        for rel in rels.iter().filter(|rel| !rel.is_empty()) {
            if lock(&self.exclusive).contains_key(*rel) {
                continue;
            }
            let mut chain = vec![rel.to_string()];
            chain.extend(self.ancestors(rel));
            claimed.extend(self.claim_chain(&chain, &mut mine));
        }
        if claimed.is_empty() {
            return;
        }
        for (folder, result) in &self.bulk(backend, claimed, cancel) {
            self.settle_slot(folder, result);
        }
        self.changed.notify_all();
    }

    fn bulk_backend(&self) -> Option<&dyn Backend> {
        match self.target {
            Side::Remote(backend) if crate::vfs::supports_create_dirs(backend, &self.root) => {
                Some(backend)
            }
            _ => None,
        }
    }

    /// The parents of `rel`, nearest first, up to (not including) a planned
    /// top-level folder.
    fn ancestors(&self, rel: &str) -> Vec<String> {
        let exclusive = lock(&self.exclusive);
        let mut ancestors = Vec::new();
        let mut current = parent_rel(rel);
        while let Some(parent) = current {
            if exclusive.contains_key(parent) {
                break;
            }
            ancestors.push(parent.to_string());
            current = parent_rel(parent);
        }
        ancestors
    }

    /// Takes the `Creating` slots of `chain` (a folder and its parents,
    /// nearest first) up to the first one that is not free: one this call
    /// holds already (`mine`), or one another asker holds or knows. Claims
    /// are always a contiguous run upwards and their creator only waits for
    /// the folder above the run, so two askers never wait for each other.
    fn claim_chain(&self, chain: &[String], mine: &mut HashSet<String>) -> Vec<String> {
        let mut slots = lock(&self.slots);
        let mut claimed = Vec::new();
        for rel in chain {
            if mine.contains(rel) {
                break;
            }
            let free = match slots.get(rel) {
                None => true,
                Some(Slot::Refused { until, .. }) => Instant::now() >= *until,
                Some(_) => false,
            };
            if !free {
                break;
            }
            slots.insert(rel.clone(), Slot::Creating);
            mine.insert(rel.clone());
            claimed.push(rel.clone());
        }
        claimed
    }

    /// Creates the claimed folders, parents first, in one request (portions
    /// of it are the backend's business). A claimed folder whose parent is
    /// not claimed has that parent ensured first, as a single creation would.
    fn bulk(
        &self,
        backend: &dyn Backend,
        mut claimed: Vec<String>,
        cancel: &AtomicBool,
    ) -> Settled {
        claimed.sort_by_key(|rel| rel.matches('/').count());
        let inside: HashSet<String> = claimed.iter().cloned().collect();
        let mut settled = Settled::new();
        let mut bases: BTreeMap<String, Result<bool, FolderError>> = BTreeMap::new();
        let mut create = Vec::new();
        for rel in claimed {
            match parent_rel(&rel).filter(|parent| !inside.contains(*parent)) {
                Some(parent) => {
                    let base = bases
                        .entry(parent.to_string())
                        .or_insert_with(|| self.ensure(parent, cancel));
                    match base {
                        Ok(_) => create.push(rel),
                        Err(error) => {
                            settled.insert(rel, Err(error.repeated()));
                        }
                    }
                }
                None => create.push(rel),
            }
        }
        if create.is_empty() {
            return settled;
        }
        let paths: Vec<String> = create.iter().map(|rel| self.path_of(rel)).collect();
        let answers = Mutex::new(Vec::new());
        let last = paths.last().cloned().unwrap_or_default();
        let request = self.attempt(&last, cancel, false, |_| {
            let results = crate::vfs::create_dirs(backend, &paths)?;
            if results.len() != paths.len() {
                return Err(io::Error::other("Ordneranlage meldet zu wenige Ergebnisse"));
            }
            *lock(&answers) = results;
            Ok(true)
        });
        if let Err(error) = request {
            for (index, rel) in create.into_iter().enumerate() {
                let error = if index == 0 {
                    error.clone()
                } else {
                    error.repeated()
                };
                settled.insert(rel, Err(error));
            }
            return settled;
        }
        let answers = std::mem::take(&mut *lock(&answers));
        for ((rel, path), answer) in create.into_iter().zip(&paths).zip(answers) {
            let parent_created = match parent_rel(&rel) {
                Some(parent) => match settled.get(parent).or_else(|| bases.get(parent)) {
                    Some(Ok(created)) => *created,
                    _ => false,
                },
                None => false,
            };
            let result = answer
                .map(|created| created || parent_created)
                .map_err(|error| FolderError::of(path, &error));
            settled.insert(rel, result);
        }
        settled
    }
}

/// A folder the bulk creation should have answered for but did not.
fn lost(rel: &str) -> FolderError {
    FolderError {
        kind: io::ErrorKind::Other,
        message: format!("Zielordner „{rel}“ anlegen: kein Ergebnis"),
        connection: false,
        congestion: false,
    }
}
