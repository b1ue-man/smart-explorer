//! Complete result membership is independent of the rows currently unfolded.
use crate::filter::CompiledFilter;
use crate::types::{FileEntry, FilterDef};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct RecursiveView {
    pub(super) rows: Vec<(usize, u32)>,
    pub(super) collapsed: HashSet<Arc<str>>,
}

impl RecursiveView {
    pub(super) fn displayed(&self, entries: &[FileEntry]) -> Vec<(usize, u32)> {
        let mut hidden_below = None;
        self.rows.iter().copied().filter(|&(index, depth)| {
            if hidden_below.is_some_and(|parent| depth > parent) {
                return false;
            }
            hidden_below = None;
            let entry = &entries[index];
            if entry.is_dir && self.collapsed.contains(&entry.key()) {
                hidden_below = Some(depth);
            }
            true
        }).collect()
    }
}

pub(super) fn relative_path<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    path.strip_prefix(root.trim_end_matches('/'))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
}

/// Link by directory identity, not by an exact spelling of a root row. A
/// streamed match whose ancestors have not arrived yet stays visible at the
/// nearest known ancestor; the next update restores its full placement.
pub(super) fn result_rows(
    entries: &[FileEntry],
    root: &str,
    filter: &FilterDef,
    mut compare: impl FnMut(usize, usize) -> std::cmp::Ordering,
) -> Vec<(usize, u32)> {
    let root = root.trim_end_matches('/');
    let directories: HashMap<_, _> = entries.iter().enumerate()
        .filter(|(_, entry)| entry.is_dir && entry.depth > 0)
        .map(|(index, entry)| (entry.path.trim_end_matches('/'), index))
        .collect();
    let mut parents = vec![None; entries.len()];
    let mut order: Vec<_> = (0..entries.len()).collect();
    order.sort_unstable_by_key(|&index| entries[index].depth);
    let mut allowed = vec![false; entries.len()];
    let mut matching = vec![false; entries.len()];
    let compiled = CompiledFilter::compile(filter);
    let mut children = vec![Vec::new(); entries.len() + 1];
    let virtual_root = entries.len();
    for &index in &order {
        let entry = &entries[index];
        if entry.depth == 0 || relative_path(&entry.path, root).is_none() {
            continue;
        }
        let mut parent = entry.parent.trim_end_matches('/');
        while parent != root && relative_path(parent, root).is_some() {
            if let Some(&candidate) = directories.get(parent) {
                if entries[candidate].depth < entry.depth {
                    parents[index] = Some(candidate);
                    break;
                }
            }
            let Some((ancestor, _)) = parent.rsplit_once('/') else { break };
            parent = ancestor;
        }
        let flags_allowed = (!entry.hidden || filter.include_hidden)
            && (!entry.system || filter.include_system);
        allowed[index] = flags_allowed && parents[index].is_none_or(|parent| allowed[parent]);
        matching[index] = allowed[index] && compiled.matches(entry, root);
        children[parents[index].unwrap_or(virtual_root)].push(index);
    }
    // Parents always have smaller depths; one reverse pass propagates results
    // even when the channel delivered entries in a different order.
    for &index in order.iter().rev() {
        if matching[index] {
            if let Some(parent) = parents[index] {
                matching[parent] = true;
            }
        }
    }
    for siblings in &mut children {
        siblings.retain(|&index| matching[index]);
        siblings.sort_unstable_by(|&left, &right| compare(left, right));
    }
    let mut stack: Vec<_> = children[virtual_root].iter().rev()
        .map(|&index| (index, 0)).collect();
    let mut rows = Vec::new();
    while let Some((index, depth)) = stack.pop() {
        let show = !entries[index].is_dir || filter.include_dirs;
        if show {
            rows.push((index, depth));
        }
        let next_depth = depth + u32::from(show);
        stack.extend(children[index].iter().rev().map(|&child| (child, next_depth)));
    }
    rows
}
