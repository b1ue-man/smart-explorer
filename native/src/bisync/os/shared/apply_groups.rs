//! Which planned actions must run one after another: actions on the same
//! path or on paths where one lies inside the other, letter case ignored. A
//! case-insensitive side sees `Foo.txt` and `foo.txt` as one file, and a file
//! that becomes a folder (or a folder that becomes a file) needs its deletion
//! before the copies below it. Within such a group deletions run first,
//! everything else keeps plan order; separate groups run concurrently.
use super::types::Action;
use std::collections::HashMap;

/// Action indices grouped as above; groups in the plan order of their first
/// action.
pub(super) fn action_groups(actions: &[Action]) -> Vec<Vec<usize>> {
    let mut sets = Sets::new(actions.len());
    // Lower-cased path → first action at that path.
    let mut at: HashMap<String, usize> = HashMap::new();
    // Lower-cased folder path → the actions found below it so far.
    let mut below: HashMap<String, Vec<usize>> = HashMap::new();
    for (position, action) in actions.iter().enumerate() {
        let key = rel_of(action).to_lowercase();
        match at.get(&key) {
            Some(&first) => sets.join(position, first),
            None => {
                at.insert(key.clone(), position);
            }
        }
        if let Some(inside) = below.get(&key) {
            for &other in inside {
                sets.join(position, other);
            }
        }
        for (end, _) in key.match_indices('/') {
            let ancestor = &key[..end];
            if let Some(&other) = at.get(ancestor) {
                sets.join(position, other);
            }
            below
                .entry(ancestor.to_string())
                .or_default()
                .push(position);
        }
    }
    let mut group_of_root: HashMap<usize, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for position in 0..actions.len() {
        let root = sets.find(position);
        let group = *group_of_root.entry(root).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group].push(position);
    }
    for group in &mut groups {
        // Stable: deletions first, each part in plan order.
        group.sort_by_key(|&position| !is_deletion(&actions[position]));
    }
    groups
}

fn is_deletion(action: &Action) -> bool {
    matches!(action, Action::DeleteA(_) | Action::DeleteB(_))
}

fn rel_of(action: &Action) -> &str {
    match action {
        Action::CopyAtoB(rel)
        | Action::CopyBtoA(rel)
        | Action::FinalizeMoveAtoB(rel)
        | Action::FinalizeMoveBtoA(rel)
        | Action::DeleteA(rel)
        | Action::DeleteB(rel)
        | Action::KeepBothAtoB(rel)
        | Action::KeepBothBtoA(rel) => rel,
    }
}

/// Union-find over action indices; a set is named by its first action.
struct Sets {
    parent: Vec<usize>,
}

impl Sets {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
        }
    }

    fn find(&mut self, mut index: usize) -> usize {
        while self.parent[index] != index {
            self.parent[index] = self.parent[self.parent[index]];
            index = self.parent[index];
        }
        index
    }

    fn join(&mut self, one: usize, other: usize) {
        let (one, other) = (self.find(one), self.find(other));
        if one != other {
            self.parent[one.max(other)] = one.min(other);
        }
    }
}
