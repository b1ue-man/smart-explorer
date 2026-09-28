//! The selected entries a walk starts from: inspected in parallel, or
//! answered by one listing of their folder when there are many.
use super::{Listed, Pending, Task, Walk, WalkEvent, WalkRoot};
use std::collections::HashMap;

/// The folder holding `path`; top-level entries resolve to their root.
pub(super) fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((parent, _)) if parent.ends_with(':') => format!("{parent}/"),
        Some((parent, _)) => parent.to_string(),
    }
}

pub(super) fn base_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
}

impl Walk<'_> {
    /// Selected entries are inspected in parallel, one round trip each. When
    /// a folder holds more selected entries than may run at once (a Ctrl+A),
    /// one listing of it answers them all instead of rounds of single stats.
    pub(super) fn queue_roots(&self, roots: &[WalkRoot]) {
        let mut groups: Vec<(String, Vec<WalkRoot>)> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for root in roots {
            let parent = parent_dir(&root.path);
            match index.get(&parent) {
                Some(&at) => groups[at].1.push(root.clone()),
                None => {
                    index.insert(parent.clone(), groups.len());
                    groups.push((parent, vec![root.clone()]));
                }
            }
        }
        let parallel = self.flow.snapshot().limit.max(1);
        for (parent, members) in groups {
            if members.len() > parallel {
                self.queue(Task::Roots { parent, members });
            } else {
                members
                    .into_iter()
                    .for_each(|root| self.queue(Task::Root(root)));
            }
        }
    }

    pub(super) fn roots(&self, parent: &str, members: Vec<WalkRoot>) {
        let mut entries: HashMap<String, Listed> = HashMap::new();
        match self.listed(parent, || self.lister.list(parent)) {
            None => return,
            // The first of several same-named entries, as in folder listings.
            Some(Ok(listed)) => listed.into_iter().for_each(|entry| {
                entries.entry(entry.name.clone()).or_insert(entry);
            }),
            // Each entry is then inspected on its own.
            Some(Err(_)) => {}
        }
        for root in members {
            if self.stopped() {
                return;
            }
            match entries.remove(base_name(&root.path)) {
                Some(listed) => {
                    if !self.root(&root, listed) {
                        return;
                    }
                }
                None => self.queue(Task::Root(root)),
            }
        }
    }

    pub(super) fn stat_root(&self, root: WalkRoot) {
        match self.listed(&root.path, || self.lister.stat(&root.path)) {
            None => {}
            Some(Ok(listed)) => {
                self.root(&root, listed);
            }
            Some(Err(error)) => {
                self.problem(&root.path, error.to_string());
            }
        }
    }

    fn root(&self, root: &WalkRoot, listed: Listed) -> bool {
        let name = base_name(&root.path);
        if crate::apptrash::excluded_name(name) {
            return self.send(WalkEvent::Omitted {
                path: root.path.clone(),
            });
        }
        if let Some(message) = self.refusal(&listed, name, &root.path) {
            return self.problem(&root.path, message);
        }
        if listed.is_dir {
            if self.options.folders
                && !self.options.flatten
                && !self.send(WalkEvent::Dir {
                    path: root.path.clone(),
                    rel: root.rel.clone(),
                })
            {
                return false;
            }
            self.queue(Task::Dir(Pending {
                path: root.path.clone(),
                rel: root.rel.clone(),
                depth: 0,
            }));
            return true;
        }
        let rel = if self.options.flatten {
            name.to_string()
        } else {
            root.rel.clone()
        };
        self.send(WalkEvent::File {
            path: root.path.clone(),
            rel,
            size: listed.size,
            mtime_ms: listed.mtime_ms,
            id: listed.id,
            md5: listed.md5,
        })
    }
}
