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
//!
//! No asker ever waits for a folder while it holds a claim that is not
//! below that folder: a single creation claims one unbroken run upwards and
//! waits only for the folder above it; `ensure_many`, whose claims come from
//! several chains, waits for nothing while it holds claims and gives a chain
//! back (its folders are then created when their files ask) when the folder
//! above it is still being created by someone else.
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
        let mut settled = self.bulk(backend, claimed, cancel, Bases::Wait);
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
        let mut chains = Vec::new();
        let mut tops = HashSet::new();
        for rel in rels.iter().filter(|rel| !rel.is_empty()) {
            if lock(&self.exclusive).contains_key(*rel) {
                continue;
            }
            let mut chain = vec![rel.to_string()];
            chain.extend(self.ancestors(rel));
            // The planned top-level folder above the chain (never part of a
            // bulk request) is ensured now, while this call holds no claim:
            // waiting for it later would wait while holding claims.
            if let Some(top) = chain.last().and_then(|last| parent_rel(last)) {
                if tops.insert(top.to_string()) {
                    // A failure stays in its slot for the chains below.
                    let _ = self.ensure(top, cancel);
                }
            }
            chains.push(chain);
        }
        let mut mine = HashSet::new();
        let mut claimed = Vec::new();
        for chain in &chains {
            claimed.extend(self.claim_chain(chain, &mut mine));
        }
        if claimed.is_empty() {
            return;
        }
        for (folder, result) in &self.bulk(backend, claimed, cancel, Bases::Peek) {
            self.settle_slot(folder, result);
        }
        self.changed.notify_all();
    }

    /// The lasting state of folder `rel` without waiting: `None` while it is
    /// being created (or nobody created it yet).
    fn settled_now(&self, rel: &str) -> Option<Result<bool, FolderError>> {
        match lock(&self.slots).get(rel) {
            Some(Slot::Ready { created }) => Some(Ok(*created)),
            Some(Slot::Failed(error)) => Some(Err(error.repeated())),
            Some(Slot::Refused { error, until }) if Instant::now() < *until => {
                Some(Err(error.repeated()))
            }
            _ => None,
        }
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
    /// not claimed has that parent ensured first, as a single creation would
    /// (`Bases::Wait`), or, with `Bases::Peek`, only looked at: a parent not
    /// settled yet gives the folder (and its claimed folders below) back.
    fn bulk(
        &self,
        backend: &dyn Backend,
        mut claimed: Vec<String>,
        cancel: &AtomicBool,
        mode: Bases,
    ) -> Settled {
        claimed.sort_by_key(|rel| rel.matches('/').count());
        let inside: HashSet<String> = claimed.iter().cloned().collect();
        let mut settled = Settled::new();
        let mut bases: BTreeMap<String, Result<bool, FolderError>> = BTreeMap::new();
        let mut released: HashSet<String> = HashSet::new();
        let mut create = Vec::new();
        for rel in claimed {
            let parent = parent_rel(&rel);
            if parent.is_some_and(|parent| released.contains(parent)) {
                released.insert(rel);
                continue;
            }
            let Some(parent) = parent.filter(|parent| !inside.contains(*parent)) else {
                create.push(rel);
                continue;
            };
            let base = match bases.get(parent) {
                Some(base) => Some(base.clone()),
                None => {
                    let base = match mode {
                        Bases::Wait => Some(self.ensure(parent, cancel)),
                        Bases::Peek => self.settled_now(parent),
                    };
                    if let Some(base) = &base {
                        bases.insert(parent.to_string(), base.clone());
                    }
                    base
                }
            };
            match base {
                Some(Ok(_)) => create.push(rel),
                Some(Err(error)) => {
                    settled.insert(rel, Err(error.repeated()));
                }
                None => {
                    released.insert(rel);
                }
            }
        }
        if !released.is_empty() {
            let mut slots = lock(&self.slots);
            for rel in &released {
                slots.remove(rel);
            }
            drop(slots);
            self.changed.notify_all();
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

/// How `bulk` treats the parent of a claimed folder it does not create.
#[derive(Clone, Copy)]
enum Bases {
    /// Ensures it (waiting for another asker): only where every claim of the
    /// call is below that parent (one unbroken run upwards).
    Wait,
    /// Takes it only when it is settled already; otherwise the folder is
    /// given back unclaimed.
    Peek,
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
