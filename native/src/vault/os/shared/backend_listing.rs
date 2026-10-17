//! Listing a vault folder (C1.8, C1.9): every entry of its content
//! directory that belongs to the vault, with its cleartext name and
//! metadata. Foreign conflict copies are listed under a display name and
//! never renamed; entries that cannot be decrypted or are damaged are
//! reported as omissions of the tolerant listing, and the strict listing
//! fails instead of returning less, so tree operations stop. Stages,
//! `dirid.c9r`, `.c9u` markers and foreign files are not part of the vault
//! and stay invisible, as in CryptoFS.
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::cryptomator::{
    decrypt_name, encrypt_name, needs_shortening, read_bounded, shortened_name, Masterkey,
    CIPHER_SUFFIX, INFLATED_NAME_FILE, INFLATED_NAME_READ_LIMIT,
};
use crate::vault::core::listing::{
    alias_name, classify_entry, conflict_candidates, AliasMap, AliasTarget, EntryClass, BAD_SIZE,
    LONG_NAME_BROKEN, LONG_NAME_MISMATCH, STORAGE_LINK, STORAGE_SPECIAL, UNDECRYPTABLE,
};
use crate::vault::core::locked_site;
use crate::vault::core::path_map::{child, components, dir_key};
use crate::vfs::{self, Backend, OmissionReason, VfsListing, VfsMeta, VfsOmission};

use super::backend::VaultBackend;
use super::backend_nodes::{invalid_data, Found, NodeKind, NodeRef};
use super::dir_cache::DirRef;

/// Node folders inspected at once on storages that allow parallel work.
const MAX_LIST_THREADS: usize = 8;

/// A listed folder and the display names of its conflict copies.
pub(super) struct Scan {
    pub(super) listing: VfsListing,
    pub(super) aliases: AliasMap,
}

struct Candidate {
    meta: VfsMeta,
    shortened: bool,
}

/// What the storage holds inside a node folder.
struct Inspected {
    children: Vec<VfsMeta>,
    inflated: Option<Result<String, &'static str>>,
}

enum Named {
    /// The stored encrypted name decrypts as it is.
    Direct(String),
    /// A foreign conflict copy: cleartext and the encrypted name inside.
    Conflict(String, String),
    /// A hidden foreign name (`._…` and the like): ignored like CryptoFS.
    Hidden,
    Undecryptable,
}

enum Listed {
    Canonical(VfsMeta),
    Alias(VfsMeta, AliasTarget),
    Omitted(VfsOmission),
    Skipped,
}

fn omission(rel: &str, reason: OmissionReason, detail: &str) -> VfsOmission {
    VfsOmission {
        rel: rel.to_string(),
        reason,
        detail: detail.to_string(),
    }
}

/// The cleartext of a stored encrypted name; conflict copies only for
/// `<…>.c9r` entries (a shortened entry's `name.c9s` holds the plain name).
fn name_entry(key: &Masterkey, encrypted: &str, parent_id: &str, conflicts: bool) -> Named {
    if let Ok(clear) = decrypt_name(key, encrypted, parent_id) {
        return Named::Direct(clear);
    }
    if !conflicts {
        return Named::Undecryptable;
    }
    if encrypted.starts_with('.') {
        return Named::Hidden;
    }
    let Some(base) = encrypted.strip_suffix(CIPHER_SUFFIX) else {
        return Named::Undecryptable;
    };
    for candidate in conflict_candidates(base) {
        let inner = format!("{candidate}{CIPHER_SUFFIX}");
        if let Ok(clear) = decrypt_name(key, &inner, parent_id) {
            return Named::Conflict(clear, inner);
        }
    }
    Named::Undecryptable
}

impl VaultBackend {
    /// `list_dir`: the tolerant listing, or an error when it is incomplete.
    pub(super) fn list_strict(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        let listing = self.list_tolerant(path)?;
        if listing.omitted.is_empty() {
            Ok(listing.entries)
        } else {
            Err(invalid_data(&format!(
                "Ordner im Tresor ist nur teilweise lesbar ({} Einträge; „Tresor prüfen“ zeigt sie)",
                listing.omitted.len()
            )))
        }
    }

    /// `list_dir_tolerant`: entries plus omissions.
    pub(super) fn list_tolerant(&self, path: &str) -> io::Result<VfsListing> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let all = components(path)?;
            let dir = self.dir_for(&all)?;
            let scan = self.scan(&dir)?;
            self.cache.put_aliases(&dir_key(&all), scan.aliases);
            Ok(scan.listing)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// Lists the content directory of `dir`.
    pub(super) fn scan(&self, dir: &DirRef) -> io::Result<Scan> {
        let stored = vfs::list_dir_tolerant(&*self.inner, &self.at(&dir.content))?;
        let mut listing = VfsListing {
            entries: Vec::new(),
            omitted: stored.omitted,
        };
        let mut candidates = Vec::new();
        for meta in stored.entries {
            let class = classify_entry(&meta.name);
            if class == EntryClass::Ignored {
                continue;
            }
            if meta.is_symlink || meta.special {
                let (reason, detail) = if meta.is_symlink {
                    (OmissionReason::Link, STORAGE_LINK)
                } else {
                    (OmissionReason::Special, STORAGE_SPECIAL)
                };
                listing.omitted.push(omission(&meta.name, reason, detail));
                continue;
            }
            let shortened = class == EntryClass::Shortened;
            candidates.push(Candidate { meta, shortened });
        }
        let inspected = self.inspect_all(dir, &candidates)?;
        let mut aliases = Vec::new();
        for (candidate, inspected) in candidates.into_iter().zip(inspected) {
            match self.entry(dir, candidate, inspected)? {
                Listed::Canonical(meta) => listing.entries.push(meta),
                Listed::Alias(meta, target) => aliases.push((meta, target)),
                Listed::Omitted(omitted) => listing.omitted.push(omitted),
                Listed::Skipped => {}
            }
        }
        let mut taken: HashSet<String> = listing
            .entries
            .iter()
            .map(|meta| meta.name.clone())
            .collect();
        aliases.sort_by(|left, right| left.1.stored.cmp(&right.1.stored));
        let mut map = AliasMap::new();
        for (mut meta, target) in aliases {
            let display = alias_name(&meta.name, &taken);
            taken.insert(display.clone());
            meta.name = display.clone();
            map.insert(display, target);
            listing.entries.push(meta);
        }
        Ok(Scan {
            listing,
            aliases: map,
        })
    }

    /// One candidate as listed entry, alias or omission.
    fn entry(
        &self,
        dir: &DirRef,
        candidate: Candidate,
        inspected: Result<Option<Inspected>, VfsOmission>,
    ) -> io::Result<Listed> {
        let inspected = match inspected {
            Ok(inspected) => inspected,
            Err(omitted) => return Ok(Listed::Omitted(omitted)),
        };
        let stored = candidate.meta.name.clone();
        let unreadable = |rel: &str, detail: &str| -> io::Result<Listed> {
            Ok(Listed::Omitted(omission(
                rel,
                OmissionReason::Unreadable,
                detail,
            )))
        };
        let encrypted = if candidate.shortened {
            match inspected
                .as_ref()
                .and_then(|inside| inside.inflated.clone())
            {
                Some(Ok(name)) if shortened_name(&name) == stored => name,
                Some(Ok(_)) => return unreadable(&stored, LONG_NAME_MISMATCH),
                _ => return unreadable(&stored, LONG_NAME_BROKEN),
            }
        } else {
            stored.clone()
        };
        let conflicts = !candidate.shortened;
        let named = self
            .session
            .with_key(|key| Ok(name_entry(key, &encrypted, &dir.id, conflicts)))?;
        let (clear, decrypting) = match named {
            Named::Direct(clear) => (clear, encrypted),
            Named::Conflict(clear, inner) => (clear, inner),
            Named::Hidden => return Ok(Listed::Skipped),
            Named::Undecryptable => return unreadable(&stored, UNDECRYPTABLE),
        };
        // Reachable under its name only when stored exactly as se stores it
        // (NFC, canonical, shortened above the threshold); else an alias.
        let expected = self
            .session
            .with_key(|key| encrypt_name(key, &clear, &dir.id))?;
        let threshold = self.session.config().shortening_threshold;
        let expected_stored = if needs_shortening(&expected, threshold) {
            shortened_name(&expected)
        } else {
            expected
        };
        let canonical = expected_stored == stored;
        let node = NodeRef {
            parent: dir.clone(),
            stored: stored.clone(),
            encrypted: decrypting.clone(),
            shortened: candidate.shortened,
            alias: !canonical,
        };
        let found = match inspected {
            None => Found {
                node,
                kind: NodeKind::File {
                    contents: candidate.meta,
                },
            },
            Some(inside) => match Self::classify(node, candidate.meta, &inside.children) {
                Ok(found) => found,
                Err(error) => return unreadable(&clear, &error.to_string()),
            },
        };
        let Ok(meta) = self.cleartext_meta(&clear, &found) else {
            return unreadable(&clear, BAD_SIZE);
        };
        Ok(if canonical {
            Listed::Canonical(meta)
        } else {
            Listed::Alias(
                meta,
                AliasTarget {
                    stored,
                    shortened: candidate.shortened,
                    encrypted: decrypting,
                },
            )
        })
    }

    /// Node folders (folders, links, shortened entries): their type files
    /// and long names, read in parallel up to the storage's parallelism.
    fn inspect_all(
        &self,
        dir: &DirRef,
        candidates: &[Candidate],
    ) -> io::Result<Vec<Result<Option<Inspected>, VfsOmission>>> {
        let folders: Vec<usize> = (0..candidates.len())
            .filter(|index| candidates[*index].meta.is_dir || candidates[*index].shortened)
            .collect();
        let results = self.in_parallel(&folders, |index| self.inspect(dir, &candidates[*index]));
        let mut out: Vec<Result<Option<Inspected>, VfsOmission>> =
            candidates.iter().map(|_| Ok(None)).collect();
        for (index, result) in folders.into_iter().zip(results) {
            let name = &candidates[index].meta.name;
            out[index] = match result {
                Some(Ok(inspected)) => Ok(Some(inspected)),
                Some(Err(error)) => Err(self.omission_of(name, error)?),
                None => return Err(io::Error::other("Auflistung im Tresor abgebrochen")),
            };
        }
        Ok(out)
    }

    fn inspect(&self, dir: &DirRef, candidate: &Candidate) -> io::Result<Inspected> {
        if !candidate.meta.is_dir {
            return Err(invalid_data(LONG_NAME_BROKEN));
        }
        let node = self.at(&child(&dir.content, &candidate.meta.name));
        let children = self.inner.list_dir(&node)?;
        let inflated = candidate.shortened.then(|| self.read_inflated(&node));
        Ok(Inspected { children, inflated })
    }

    /// The full encrypted name in `name.c9s` (at most 10 KiB, CryptoFS).
    fn read_inflated(&self, node: &str) -> Result<String, &'static str> {
        let reader = self
            .inner
            .open_read(&format!("{node}/{INFLATED_NAME_FILE}"))
            .map_err(|_| LONG_NAME_BROKEN)?;
        let bytes = read_bounded(reader, INFLATED_NAME_READ_LIMIT)
            .ok()
            .flatten()
            .ok_or(LONG_NAME_BROKEN)?;
        let name = String::from_utf8(bytes).map_err(|_| LONG_NAME_BROKEN)?;
        if name.len() > CIPHER_SUFFIX.len() && name.ends_with(CIPHER_SUFFIX) {
            Ok(name)
        } else {
            Err(LONG_NAME_BROKEN)
        }
    }

    /// An entry that could not be inspected is an omission; a locked vault
    /// or a broken connection fails the listing.
    fn omission_of(&self, stored: &str, error: io::Error) -> io::Result<VfsOmission> {
        if locked_site(&error).is_some() || self.session.is_locked() {
            return Err(self.session.relock(error));
        }
        let reason = match error.kind() {
            io::ErrorKind::NotFound => OmissionReason::Vanished,
            io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidData => {
                OmissionReason::Unreadable
            }
            _ => return Err(error),
        };
        Ok(omission(stored, reason, &error.to_string()))
    }

    /// `work` for every item, on up to `parallelism()` threads (at most
    /// `MAX_LIST_THREADS`); `None` only where a worker died.
    fn in_parallel<T: Sync, R: Send>(
        &self,
        items: &[T],
        work: impl Fn(&T) -> R + Sync,
    ) -> Vec<Option<R>> {
        let width = self
            .inner
            .parallelism()
            .clamp(1, MAX_LIST_THREADS)
            .min(items.len());
        if width <= 1 {
            return items.iter().map(|item| Some(work(item))).collect();
        }
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
        std::thread::scope(|scope| {
            for _ in 0..width {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    let result = work(item);
                    if let Some(slot) = results
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get_mut(index)
                    {
                        *slot = Some(result);
                    }
                });
            }
        });
        results.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}
