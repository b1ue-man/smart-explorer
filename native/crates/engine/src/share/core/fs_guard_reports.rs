//! Provider reports keep their fast path; private entries never leave Share.
use super::super::fs_host_policy::TargetPolicy;
use crate::analytics::{DuplicateReport, ScanOutcome, SizeNode};
use std::{io, vec::IntoIter};

/// Logical and occupied bytes a hidden subtree takes from its parents.
#[derive(Clone, Copy, Default)]
struct Removed {
    size: u64,
    allocated: u64,
}
impl Removed {
    fn add(&mut self, other: Removed) {
        self.size = self.size.saturating_add(other.size);
        self.allocated = self.allocated.saturating_add(other.allocated);
    }
}

trait Node: Sized {
    fn name(&self) -> &str;
    fn bytes(&self) -> Removed;
    fn subtract(&mut self, removed: Removed);
    fn children(&mut self) -> &mut Vec<Self>;
}
impl Node for SizeNode {
    fn name(&self) -> &str {
        &self.name
    }
    fn bytes(&self) -> Removed {
        Removed {
            size: self.size,
            allocated: self.allocated,
        }
    }
    fn subtract(&mut self, removed: Removed) {
        self.size = self.size.saturating_sub(removed.size);
        self.allocated = self.allocated.saturating_sub(removed.allocated);
    }
    fn children(&mut self) -> &mut Vec<Self> {
        &mut self.children
    }
}
/// A wire node without occupied bytes occupies its size.
impl Node for crate::agent_proto::WireNode {
    fn name(&self) -> &str {
        &self.name
    }
    fn bytes(&self) -> Removed {
        Removed {
            size: self.size,
            allocated: self.allocated.unwrap_or(self.size),
        }
    }
    fn subtract(&mut self, removed: Removed) {
        self.size = self.size.saturating_sub(removed.size);
        self.allocated = self
            .allocated
            .map(|bytes| bytes.saturating_sub(removed.allocated));
    }
    fn children(&mut self) -> &mut Vec<Self> {
        &mut self.children
    }
}

struct Frame<N> {
    node: N,
    path: String,
    children: IntoIter<N>,
    removed: Removed,
}
fn frame<N: Node>(mut node: N, path: String) -> Frame<N> {
    let children = std::mem::take(node.children()).into_iter();
    Frame {
        node,
        path,
        children,
        removed: Removed::default(),
    }
}
fn tree<N: Node>(node: N, root: &str, policy: &TargetPolicy) -> io::Result<N> {
    let mut stack = vec![frame(node, root.to_owned())];
    loop {
        let current = stack
            .last_mut()
            .ok_or_else(|| io::Error::other("Leerer Analyse-Filter"))?;
        if let Some(child) = current.children.next() {
            crate::vfs::validate_child_name(child.name())?;
            let path = format!("{}/{}", current.path.trim_end_matches('/'), child.name());
            if policy.visible(&path, false) {
                stack.push(frame(child, path));
            } else {
                current.removed.add(child.bytes());
            }
            continue;
        }
        let Some(mut done) = stack.pop() else {
            return Err(io::Error::other("Leerer Analyse-Filter"));
        };
        done.node.subtract(done.removed);
        if let Some(parent) = stack.last_mut() {
            parent.removed.add(done.removed);
            parent.node.children().push(done.node);
        } else {
            return Ok(done.node);
        }
    }
}

pub(super) fn scan(
    mut outcome: ScanOutcome,
    root: &str,
    policy: &TargetPolicy,
) -> io::Result<ScanOutcome> {
    if let Some(node) = outcome.tree.take() {
        outcome.tree = Some(tree(node, root, policy)?);
    }
    outcome.issues.retain(|issue| {
        policy.visible(&issue.path, false) && !super::super::fs_policy::private_path(&issue.detail)
    });
    outcome
        .protected
        .retain(|area| policy.visible(&area.area, false));
    outcome
        .notes
        .retain(|text| !super::super::fs_policy::private_path(text));
    Ok(outcome)
}
pub(super) fn wire(
    node: crate::agent_proto::WireNode,
    root: &str,
    policy: &TargetPolicy,
) -> io::Result<crate::agent_proto::WireNode> {
    tree(node, root, policy)
}
pub(super) fn duplicates(mut report: DuplicateReport, policy: &TargetPolicy) -> DuplicateReport {
    for group in &mut report.groups {
        group.items.retain(|item| {
            policy.visible(&item.path, false) && !super::super::fs_policy::private_name(&item.name)
        });
        group.reclaimable = crate::analytics::DuplicateGroup::reclaimable_of(&group.items);
    }
    report.groups.retain(|group| group.items.len() > 1);
    report.summary.groups = report.groups.len() as u64;
    report
        .summary
        .protected
        .retain(|area| policy.visible(&area.area, false));
    report
        .summary
        .errors
        .retain(|text| !super::super::fs_policy::private_path(text));
    report
        .summary
        .limits
        .retain(|text| !super::super::fs_policy::private_path(text));
    if report
        .root_error
        .as_ref()
        .is_some_and(|text| super::super::fs_policy::private_path(text))
    {
        report.root_error = Some("Pfad ist nicht freigegeben".into());
    }
    report
}

#[cfg(test)]
#[path = "fs_guard_reports_task_tests.rs"]
mod task_tests;
