//! Path-addressable names for Google Drive siblings that share one name.
//!
//! Drive keys objects by id and allows several children with the same name in
//! one folder. Every path-based consumer (browse walk, analytics, sync)
//! requires unique child names, so a raw listing with duplicates used to fail
//! the whole directory. The listing now keeps the canonical sibling under its
//! plain name and renders every further sibling as
//! `"<name> [drive-id <id prefix>]"`; `find_child` understands that marker and
//! addresses the exact object again. Canonical = newest `modifiedTime`, ties
//! broken by the smaller id, the same rule the duplicate cleanup planner uses.
use crate::vfs::VfsMeta;
use std::collections::{HashMap, HashSet};

pub(super) const MARKER_PREFIX: &str = " [drive-id ";
pub(super) const MARKER_SUFFIX: &str = "]";
pub(super) const ID_PREFIX_LEN: usize = 8;

/// `(plain name, id prefix)` when `name` carries a duplicate marker.
pub(super) fn parse_marker(name: &str) -> Option<(&str, &str)> {
    let body = name.strip_suffix(MARKER_SUFFIX)?;
    let start = body.rfind(MARKER_PREFIX)?;
    let plain = &body[..start];
    let prefix = &body[start + MARKER_PREFIX.len()..];
    if plain.is_empty()
        || prefix.is_empty()
        || !prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some((plain, prefix))
}

pub(super) fn marker_name(plain: &str, id: &str) -> String {
    let prefix: String = id.chars().take(ID_PREFIX_LEN).collect();
    format!("{plain}{MARKER_PREFIX}{prefix}{MARKER_SUFFIX}")
}

fn full_marker_name(plain: &str, id: &str) -> String {
    format!("{plain}{MARKER_PREFIX}{id}{MARKER_SUFFIX}")
}

/// Newest first, then the smaller id, so the choice does not depend on the
/// order Drive returned the siblings in.
fn canonical_order(left: &VfsMeta, right: &VfsMeta) -> std::cmp::Ordering {
    right
        .mtime_ms
        .cmp(&left.mtime_ms)
        .then_with(|| left.id.cmp(&right.id))
}

/// Rewrite duplicate sibling names so every returned name is unique and
/// re-resolvable. Entries without an id cannot be addressed exactly and keep
/// their raw name (a later walk guard still rejects them fail-closed).
pub(super) fn disambiguate(entries: Vec<VfsMeta>) -> Vec<VfsMeta> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for entry in &entries {
        *counts.entry(entry.name.as_str()).or_default() += 1;
    }
    if counts.values().all(|count| *count == 1) {
        return entries;
    }
    let raw_names: HashSet<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let mut groups: HashMap<String, Vec<VfsMeta>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for entry in entries {
        let name = entry.name.clone();
        let group = groups.entry(name.clone()).or_default();
        if group.is_empty() {
            order.push(name);
        }
        group.push(entry);
    }
    let mut out = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    for name in order {
        let mut group = groups.remove(&name).unwrap_or_default();
        if group.len() == 1 {
            let entry = group.remove(0);
            used.insert(entry.name.clone());
            out.push(entry);
            continue;
        }
        group.sort_by(canonical_order);
        for (index, mut entry) in group.into_iter().enumerate() {
            if index > 0 {
                if let Some(id) = entry.id.clone().filter(|id| !id.is_empty()) {
                    let short = marker_name(&name, &id);
                    let long = full_marker_name(&name, &id);
                    entry.name = if !raw_names.contains(&short) && !used.contains(&short) {
                        short
                    } else {
                        long
                    };
                }
            }
            used.insert(entry.name.clone());
            out.push(entry);
        }
    }
    out
}

/// One same-name sibling as Drive returned it for a lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Sibling {
    pub id: String,
    pub mtime_ms: i64,
}

/// The canonical sibling for a plain name (newest, then smaller id).
pub(super) fn select_canonical(siblings: &[Sibling]) -> Option<String> {
    siblings
        .iter()
        .min_by(|left, right| {
            right
                .mtime_ms
                .cmp(&left.mtime_ms)
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|sibling| sibling.id.clone())
}

/// The sibling addressed by a marker's id prefix. An ambiguous prefix fails
/// closed instead of guessing.
pub(super) fn select_by_prefix(siblings: &[Sibling], prefix: &str) -> Result<Option<String>, String> {
    let matches: Vec<&Sibling> = siblings
        .iter()
        .filter(|sibling| sibling.id.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.id.clone())),
        _ => Err(format!(
            "Drive duplicate marker {prefix:?} matches more than one sibling"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, id: &str, mtime_ms: i64, is_dir: bool) -> VfsMeta {
        VfsMeta {
            name: name.into(),
            is_dir,
            id: Some(id.into()),
            mtime_ms,
            ..Default::default()
        }
    }

    #[test]
    fn lan_cleanup_task_unique_names_are_untouched() {
        let entries = vec![meta("a", "1", 1, true), meta("b", "2", 2, false)];
        let out = disambiguate(entries);
        let names: Vec<&str> = out.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(out[0].id.as_deref(), Some("1"));
    }

    #[test]
    fn lan_cleanup_task_duplicate_folders_keep_newest_plain_and_mark_the_rest() {
        let entries = vec![
            meta("X", "olderid00001", 10, true),
            meta("X", "newerid00002", 20, true),
            meta("y", "3", 5, false),
        ];
        let out = disambiguate(entries);
        let names: Vec<&str> = out.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["X", "X [drive-id olderid0]", "y"]);
        assert_eq!(out[0].id.as_deref(), Some("newerid00002"));
        let (plain, prefix) = parse_marker(&out[1].name).unwrap();
        assert_eq!(plain, "X");
        assert_eq!(prefix, "olderid0");
    }

    #[test]
    fn lan_cleanup_task_equal_timestamps_break_ties_by_id() {
        let entries = vec![meta("f", "b", 1, false), meta("f", "a", 1, false)];
        let out = disambiguate(entries);
        assert_eq!(out[0].id.as_deref(), Some("a"));
        assert_eq!(out[1].name, "f [drive-id b]");
    }

    #[test]
    fn lan_cleanup_task_marker_collision_with_literal_name_uses_full_id() {
        let entries = vec![
            meta("f", "abcdefgh1", 2, false),
            meta("f", "abcdefgh2", 1, false),
            meta("f [drive-id abcdefgh]", "literal", 1, false),
        ];
        let out = disambiguate(entries);
        let names: Vec<&str> = out.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            ["f", "f [drive-id abcdefgh2]", "f [drive-id abcdefgh]"]
        );
    }

    #[test]
    fn lan_cleanup_task_entries_without_id_keep_their_raw_name() {
        let mut second = meta("f", "", 1, false);
        second.id = None;
        let out = disambiguate(vec![meta("f", "a", 2, false), second]);
        assert_eq!(out[1].name, "f");
    }

    #[test]
    fn lan_cleanup_task_parse_marker_rejects_non_markers() {
        assert!(parse_marker("plain").is_none());
        assert!(parse_marker(" [drive-id abc]").is_none());
        assert!(parse_marker("x [drive-id ]").is_none());
        assert!(parse_marker("x [drive-id a b]").is_none());
        assert_eq!(parse_marker("x [drive-id ab_c-1]"), Some(("x", "ab_c-1")));
    }

    #[test]
    fn lan_cleanup_task_canonical_and_prefix_selection() {
        let siblings = vec![
            Sibling {
                id: "abcd1111".into(),
                mtime_ms: 1,
            },
            Sibling {
                id: "abcd2222".into(),
                mtime_ms: 2,
            },
        ];
        assert_eq!(select_canonical(&siblings).as_deref(), Some("abcd2222"));
        assert_eq!(
            select_by_prefix(&siblings, "abcd1").unwrap().as_deref(),
            Some("abcd1111")
        );
        assert_eq!(select_by_prefix(&siblings, "zzz").unwrap(), None);
        assert!(select_by_prefix(&siblings, "abcd").is_err());
        assert_eq!(select_canonical(&[]), None);
    }
}
