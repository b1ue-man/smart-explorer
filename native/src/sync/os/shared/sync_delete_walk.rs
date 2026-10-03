//! Protected literal walks and a fresh normalized source-absence check.
use super::imp::{require_plain_directory, WalkBudget};
use crate::bisync::{KeyPolicy, OmissionKind, SyncOmissions};
use crate::vfs::{Backend, VfsMeta};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Entry {
    pub(super) path: String,
    pub(super) rel: String,
    pub(super) meta: VfsMeta,
}
fn child(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}
fn gate(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "mirror canceled",
        ))
    } else {
        Ok(())
    }
}

pub(super) fn walk(
    backend: &dyn Backend,
    root: &str,
    keys: KeyPolicy,
    omissions: &mut SyncOmissions,
    cancel: &AtomicBool,
) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut queue = VecDeque::from([(root.to_string(), String::new(), 0usize)]);
    let mut budget = WalkBudget::default();
    while let Some((path, rel, depth)) = queue.pop_front() {
        gate(cancel)?;
        if omissions.contains(&rel) && !rel.is_empty() {
            continue;
        }
        let listing = (|| {
            require_plain_directory(backend, &path, false)?;
            crate::vfs::list_dir_tolerant(backend, &path)
        })();
        let listing = match listing {
            Ok(listing) => listing,
            Err(error) if rel.is_empty() => return Err(error),
            Err(error) => {
                let kind = crate::bisync::apply_boundary::omitted(&error)
                    .unwrap_or(OmissionKind::Unreadable);
                omissions.record_kind(&rel, kind, kind.reported_by_default());
                continue;
            }
        };
        for omitted in &listing.omitted {
            let relative = if crate::vfs::validate_child_name(&omitted.rel).is_ok() {
                child(&rel, &omitted.rel)
            } else {
                rel.clone()
            };
            let kind: OmissionKind = omitted.reason.into();
            omissions.record_kind(&relative, kind, kind.reported_by_default());
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        for meta in &listing.entries {
            *counts.entry(keys.key(&meta.name).into_owned()).or_default() += 1;
        }
        for meta in listing.entries {
            gate(cancel)?;
            if crate::vfs::validate_child_name(&meta.name).is_err() {
                omissions.record_kind(&rel, OmissionKind::NotRepresentable, true);
                continue;
            }
            let child_rel = child(&rel, &meta.name);
            budget
                .record(&child_rel, depth + 1)
                .map_err(io::Error::other)?;
            if counts
                .get(keys.key(&meta.name).as_ref())
                .copied()
                .unwrap_or(0)
                != 1
            {
                omissions.record_kind(&child_rel, OmissionKind::NameImpossibleOnTarget, true);
                continue;
            }
            let child_path = crate::vfs::sync_child_path(backend, &path, &meta.name)?;
            match crate::bisync::snapshot_policy::protected(
                backend,
                root,
                &child_path,
                &meta,
                false,
            ) {
                Ok(Some(kind)) => {
                    omissions.record_kind(&child_rel, kind, kind.reported_by_default());
                    continue;
                }
                Err(_) => {
                    omissions.record_kind(&child_rel, OmissionKind::Unreadable, true);
                    continue;
                }
                Ok(None) => {}
            }
            if meta.is_dir {
                queue.push_back((child_path.clone(), child_rel.clone(), depth + 1));
            }
            entries.push(Entry {
                path: child_path,
                rel: child_rel,
                meta,
            });
        }
    }
    Ok(entries)
}

/// Each component is compared under the pair's key policy. An encoded URI
/// basename or a differently spelled source must never mean "missing".
pub(super) fn missing(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    keys: KeyPolicy,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    crate::bisync::apply_boundary::normalized_missing(backend, root, rel, keys, false, cancel)
}
