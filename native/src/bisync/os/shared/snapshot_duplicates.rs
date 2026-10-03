//! Keep same-name files out of ordinary path-based planning until both sides
//! can be compared. Browsing aliases never become independent sync files.
use crate::vfs::VfsMeta;
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::sync::atomic::Ordering;

use super::snapshot_dir::WalkContext;

pub(super) type DuplicateGroups = BTreeMap<String, Vec<VfsMeta>>;

pub(super) fn observe(
    ctx: &WalkContext<'_>,
    dir: &str,
    dir_rel: &str,
    entries: Vec<VfsMeta>,
) -> io::Result<Vec<VfsMeta>> {
    let Some(observations) = ctx.duplicates else {
        return Ok(entries);
    };
    if !ctx.be.has_duplicate_file_names() {
        return Ok(entries);
    }
    let mut groups: BTreeMap<String, Vec<VfsMeta>> = BTreeMap::new();
    for entry in entries {
        if crate::vfs::validate_child_name(&entry.name).is_err() {
            super::snapshot_dir::record_omission(
                ctx,
                &super::snapshot_dir::literal_child(dir_rel, &entry.name),
                super::OmissionKind::NotRepresentable,
                true,
            );
            continue;
        }
        groups.entry(entry.name.clone()).or_default().push(entry);
    }
    let mut unique = Vec::new();
    for (name, group) in groups {
        if group.len() == 1 {
            unique.extend(group);
            continue;
        }
        let path = crate::vfs::sync_child_path(ctx.be, dir, &name)?;
        let rel = super::snapshot_dir::literal_child(dir_rel, &name);
        let mut ids = HashSet::new();
        for entry in &group {
            if ctx.cancel.load(Ordering::Relaxed) {
                return Ok(unique);
            }
            if ctx.nodes.fetch_add(1, Ordering::Relaxed) >= ctx.limits.walk_entries
                || ctx
                    .text_bytes
                    .fetch_add(rel.len() as u64, Ordering::Relaxed)
                    > ctx.limits.walk_text_bytes.saturating_sub(rel.len() as u64)
            {
                return Err(io::Error::other(
                    "sync duplicate observations exceed collection budget",
                ));
            }
            if entry
                .id
                .as_deref()
                .is_none_or(|id| id.is_empty() || !ids.insert(id))
            {
                return Err(io::Error::other(
                    "Drive-Duplikate haben keine eindeutigen Datei-IDs",
                ));
            }
        }
        let filtered = ctx.filter.ignored(&rel, false)
            || group.iter().any(|m| {
                (!ctx.filter.include_hidden && m.hidden)
                    || !ctx.filter.size_age_ok(m.size, m.mtime_ms)
            });
        let protected = group.iter().any(|m| m.is_dir || m.is_symlink || m.special)
            || crate::apptrash::excluded_name(&name)
            || crate::apptrash::hidden_app_folders_in(dir);
        if filtered
            || protected
            || super::paths::is_engine_name(&name)
            || crate::vfs::is_staging_name(&name)
            || super::snapshot_policy::own_path(ctx.be, &path)
        {
            if let Some(omissions) = ctx.omissions {
                omissions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .record_kind(
                        &rel,
                        if filtered {
                            super::OmissionKind::Filtered
                        } else if super::paths::is_engine_name(&name)
                            || crate::vfs::is_staging_name(&name)
                            || super::snapshot_policy::own_path(ctx.be, &path)
                        {
                            super::OmissionKind::OwnFile
                        } else if group.iter().any(|m| m.is_symlink) {
                            super::OmissionKind::Link
                        } else if group.iter().any(|m| m.special) {
                            super::OmissionKind::Special
                        } else {
                            super::OmissionKind::NameImpossibleOnTarget
                        },
                        !filtered && protected,
                    );
            }
            continue;
        }
        observations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(rel, group);
    }
    Ok(unique)
}
