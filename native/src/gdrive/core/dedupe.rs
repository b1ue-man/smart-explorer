use super::GDriveBackend;
use crate::vfs::{DedupeCandidate, VfsMeta, VfsResult};
use std::collections::{HashMap, HashSet};
use std::io;

const MAX_NODES: u64 = 1_000_000;
const MAX_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DEPTH: usize = 512;

impl GDriveBackend {
    /// Produce an exact ID-addressed duplicate cleanup plan. Planning is fully
    /// read-only so the sync deletion guard can reject the complete operation
    /// before any file is copied or removed.
    pub(super) fn plan_duplicate_cleanup(
        &self,
        root: &str,
        keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<DedupeCandidate>> {
        let root = super::core::norm(root);
        let mut stack = vec![(root.clone(), String::new(), 0usize)];
        let mut plan = Vec::new();
        let mut nodes = 0u64;
        let mut text_bytes = root.len() as u64;

        while let Some((dir, dir_rel, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(invalid("Drive duplicate cleanup exceeds its depth budget"));
            }
            let mut groups: HashMap<String, Vec<VfsMeta>> = HashMap::new();
            for metadata in crate::vfs::Backend::list_dir(self, &dir)? {
                crate::vfs::validate_child_name(&metadata.name)?;
                nodes = nodes.saturating_add(1);
                text_bytes = text_bytes.saturating_add(metadata.name.len() as u64);
                if nodes > MAX_NODES || text_bytes > MAX_TEXT_BYTES {
                    return Err(invalid(
                        "Drive duplicate cleanup exceeds its collection budget",
                    ));
                }
                if metadata.is_symlink {
                    return Err(invalid("Drive cleanup encountered a link-like entry"));
                }
                groups
                    .entry(metadata.name.clone())
                    .or_default()
                    .push(metadata);
            }

            for (name, group) in groups {
                let rel = join(&dir_rel, &name);
                let path = join(&dir, &name);
                let directories = group.iter().filter(|entry| entry.is_dir).count();
                if directories > 0 {
                    // Path resolution cannot safely select one of duplicate
                    // same-name folders, or distinguish a folder/file collision.
                    if group.len() != 1 || directories != 1 {
                        return Err(invalid(format!(
                            "Drive has an ambiguous duplicate folder path: {path}"
                        )));
                    }
                    stack.push((path, rel, depth + 1));
                    continue;
                }
                plan.extend(select_file_candidates(&path, &rel, group, keep(&rel))?);
                if plan.len() as u64 > MAX_NODES {
                    return Err(invalid("Drive duplicate cleanup plan is too large"));
                }
            }
        }
        Ok(plan)
    }
}

fn select_file_candidates(
    path: &str,
    rel: &str,
    mut group: Vec<VfsMeta>,
    keep_one: bool,
) -> VfsResult<Vec<DedupeCandidate>> {
    if group.len() < 2 {
        return Ok(Vec::new());
    }
    let mut ids = HashSet::new();
    for metadata in &group {
        let id = metadata
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid(format!("Drive duplicate has no stable ID: {rel}")))?;
        if !ids.insert(id.to_string()) {
            return Err(invalid(format!(
                "Drive returned the same duplicate ID more than once: {rel}"
            )));
        }
    }
    // Keep the newest item, breaking equal timestamps by stable ID so the plan
    // does not depend on Drive's listing order.
    group.sort_by(|left, right| {
        right
            .mtime_ms
            .cmp(&left.mtime_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    let skip = usize::from(keep_one);
    Ok(group
        .into_iter()
        .skip(skip)
        .map(|metadata| DedupeCandidate {
            path: path.to_string(),
            id: metadata.id,
        })
        .collect())
}

fn join(parent: &str, child: &str) -> String {
    if parent.is_empty() || parent == "/" {
        format!("{}{child}", if parent == "/" { "/" } else { "" })
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, mtime_ms: i64) -> VfsMeta {
        VfsMeta {
            id: Some(id.into()),
            mtime_ms,
            ..Default::default()
        }
    }

    #[test]
    fn duplicate_plan_keeps_newest_by_stable_id() {
        let plan = select_file_candidates(
            "root/x",
            "x",
            vec![file("z", 1), file("b", 2), file("a", 2)],
            true,
        )
        .unwrap();
        let ids: Vec<_> = plan.iter().filter_map(|item| item.id.as_deref()).collect();
        assert_eq!(ids, vec!["b", "z"]);
    }

    #[test]
    fn orphan_plan_removes_every_duplicate() {
        let plan =
            select_file_candidates("root/x", "x", vec![file("a", 1), file("b", 2)], false).unwrap();
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn duplicate_without_id_fails_closed() {
        assert!(select_file_candidates(
            "root/x",
            "x",
            vec![file("a", 1), VfsMeta::default()],
            true,
        )
        .is_err());
    }
}
