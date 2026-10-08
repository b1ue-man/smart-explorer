//! Which entries of the account-wide Drive change feed concern one sync
//! root. A change counts when its object is known below the root (edit,
//! trash, removal, move out) or when its new parent chain leads into the
//! root (creation, move in). Known objects come from the account cache the
//! sync listings persist (every folder and every uniquely named file) plus
//! objects this subscription saw arriving below the root. Unknown parents are
//! looked up once per subscription; any lookup failure counts as relevant.
//!
//! Limit: removing a same-name duplicate file below the root that no
//! listing could bind to a path is not recognized here; the job's periodic
//! verification run covers it.
use std::collections::{HashMap, HashSet};
use std::io;

use super::core::{cloud_urlenc, norm};
use super::GDriveBackend;
use crate::vfs::{ChangeKind, VfsChange};

/// Parent hops followed before an unknown chain counts as relevant (Drive
/// folders are far shallower; the bound only stops a cyclic answer).
const MAX_HOPS: usize = 256;

pub(super) struct ChangeScope {
    root_key: String,
    root_id: String,
    /// Folder id → whether it lies below the root, learned from lookups.
    ancestry: HashMap<String, bool>,
    /// Objects created or moved below the root while subscribed.
    arrived: HashSet<String>,
}

impl ChangeScope {
    pub(super) fn new(backend: &GDriveBackend, root: &str) -> io::Result<Self> {
        let root_id = backend.actual_parent_id(&backend.resolve(root)?)?;
        Ok(Self {
            root_key: norm(root),
            root_id,
            ancestry: HashMap::new(),
            arrived: HashSet::new(),
        })
    }

    /// The feed of this account covers the root completely only in My Drive:
    /// `changes.list` without `driveId` does not report shared-drive items.
    pub(super) fn feed_complete(&self, backend: &GDriveBackend) -> bool {
        backend
            .narrowed_listing_corpus(&self.root_id)
            .is_ok_and(|corpus| corpus == "&corpora=user")
    }

    fn known_below_root(&self, backend: &GDriveBackend) -> HashSet<String> {
        let prefix = format!("{}/", self.root_key);
        let below = |key: &str| key == self.root_key || key.starts_with(&prefix);
        let mut known: HashSet<String> = HashSet::from([self.root_id.clone()]);
        let caches = super::cache::load_account(
            &super::binding_store::account_cache_path(&backend.drive_account_key),
            &super::cache::cache_path(),
        );
        known.extend(
            caches
                .hints
                .ids
                .iter()
                .filter(|(key, _)| below(key))
                .map(|(_, id)| id.clone()),
        );
        if let Ok(ids) = backend.ids_guard() {
            known.extend(
                ids.iter()
                    .filter(|(key, _)| below(key))
                    .map(|(_, id)| id.clone()),
            );
        }
        known
    }

    /// Whether any change of this batch can affect the root.
    pub(super) fn relevant(&mut self, backend: &GDriveBackend, changes: &[VfsChange]) -> bool {
        if changes.is_empty() {
            return false;
        }
        if self.root_key.is_empty() {
            return true;
        }
        let known = self.known_below_root(backend);
        self.relevant_with(changes, &known, &mut |id: &str| parent_of(backend, id))
    }

    /// The decision for one batch, given the objects known below the root
    /// and a lookup of an object's parent (`None`: top of the drive).
    pub(super) fn relevant_with(
        &mut self,
        changes: &[VfsChange],
        known: &HashSet<String>,
        lookup: &mut dyn FnMut(&str) -> io::Result<Option<String>>,
    ) -> bool {
        // A moved or renamed folder changes the ancestry of its subtree.
        if changes.iter().any(is_folder) {
            self.ancestry.clear();
        }
        let mut relevant = false;
        for change in changes {
            let id = change.id.as_deref().unwrap_or("");
            if known.contains(id) || self.arrived.contains(id) || id == self.root_id {
                relevant = true;
                continue;
            }
            let Some(parent) = change.parent_id.as_deref() else {
                // A removal without metadata of an object never bound below
                // the root (see the module limit).
                continue;
            };
            let inside = self.below(parent, known, lookup).unwrap_or(true);
            if inside {
                relevant = true;
                if change.kind == ChangeKind::Upsert {
                    self.arrived.insert(id.to_string());
                }
            }
        }
        relevant
    }

    /// Follows `parent` upwards until the root, a known object below it, or
    /// the top of the drive.
    fn below(
        &mut self,
        parent: &str,
        known: &HashSet<String>,
        lookup: &mut dyn FnMut(&str) -> io::Result<Option<String>>,
    ) -> io::Result<bool> {
        let mut chain = Vec::new();
        let mut current = parent.to_string();
        let mut answer = None;
        for _ in 0..MAX_HOPS {
            if current == self.root_id
                || known.contains(&current)
                || self.arrived.contains(&current)
            {
                answer = Some(true);
                break;
            }
            if let Some(cached) = self.ancestry.get(&current) {
                answer = Some(*cached);
                break;
            }
            chain.push(current.clone());
            match lookup(&current)? {
                Some(next) => current = next,
                None => {
                    answer = Some(false);
                    break;
                }
            }
        }
        let answer = answer.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Drive parent chain does not end",
            )
        })?;
        for id in chain {
            self.ancestry.insert(id, answer);
        }
        Ok(answer)
    }
}

/// The first parent of a Drive object; `None` at the top of the drive.
fn parent_of(backend: &GDriveBackend, id: &str) -> io::Result<Option<String>> {
    let json = backend.get_json(&backend.api_url(&format!(
        "files/{}?fields=id,parents&supportsAllDrives=true",
        cloud_urlenc(id)
    )))?;
    match json["parents"]
        .as_array()
        .and_then(|parents| parents.first())
    {
        Some(next) => next
            .as_str()
            .filter(|id| !id.is_empty())
            .map(|id| Some(id.to_string()))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Drive parent id is invalid")
            }),
        None => Ok(None),
    }
}

fn is_folder(change: &VfsChange) -> bool {
    change.meta.as_ref().is_some_and(|meta| meta.is_dir)
}

#[cfg(test)]
impl ChangeScope {
    pub(super) fn for_test(root_key: &str, root_id: &str) -> Self {
        Self {
            root_key: root_key.to_string(),
            root_id: root_id.to_string(),
            ancestry: HashMap::new(),
            arrived: HashSet::new(),
        }
    }
}

#[cfg(test)]
#[path = "sync_transparency_task_scope_tests.rs"]
mod sync_transparency_task_scope_tests;
