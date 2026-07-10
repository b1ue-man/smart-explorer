use std::collections::{HashMap, HashSet};
use std::io;

use super::wire::FsWalkNode;

pub(super) const WALK_BATCH_NODES: usize = 256;
pub(super) const MAX_WALK_NODES: usize = 2_000_000;
pub(super) const MAX_WALK_NAME_BYTES: usize = 128 * 1024 * 1024;
pub(super) const MAX_WALK_DEPTH: usize = 512;
const MAX_NAME_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct WalkTotals {
    pub(super) files: u64,
    pub(super) dirs: u64,
    pub(super) bytes: u64,
}

impl WalkTotals {
    pub(super) fn observe(&mut self, node: &FsWalkNode) -> io::Result<()> {
        if node.is_dir {
            self.dirs = self.dirs.checked_add(1).ok_or_else(count_overflow)?;
        } else {
            self.files = self.files.checked_add(1).ok_or_else(count_overflow)?;
            self.bytes = self
                .bytes
                .checked_add(node.size)
                .ok_or_else(count_overflow)?;
        }
        Ok(())
    }

    pub(super) fn nodes(self) -> u64 {
        self.files.saturating_add(self.dirs)
    }
}

#[derive(Default)]
pub(super) struct TreeAssembler {
    seen: HashSet<u64>,
    waiting: HashMap<u64, Vec<BuiltNode>>,
    root: Option<crate::agent_proto::WireNode>,
    totals: WalkTotals,
    name_bytes: usize,
}

struct BuiltNode {
    wire: crate::agent_proto::WireNode,
    depth: usize,
}

impl TreeAssembler {
    pub(super) fn push_batch(
        &mut self,
        nodes: Vec<FsWalkNode>,
        totals: WalkTotals,
    ) -> io::Result<()> {
        if nodes.len() > WALK_BATCH_NODES {
            return Err(invalid("peer tree batch exceeds node limit"));
        }
        for node in nodes {
            self.push(node)?;
        }
        if self.totals != totals {
            return Err(invalid("peer tree progress totals do not match nodes"));
        }
        Ok(())
    }

    pub(super) fn push(&mut self, node: FsWalkNode) -> io::Result<()> {
        if self.root.is_some() || self.seen.len() >= MAX_WALK_NODES {
            return Err(invalid("peer tree has too many or post-root nodes"));
        }
        validate_name(&node.name, node.id == 0)?;
        self.name_bytes = self
            .name_bytes
            .checked_add(node.name.len())
            .filter(|n| *n <= MAX_WALK_NAME_BYTES)
            .ok_or_else(|| invalid("peer tree name data exceeds safety limit"))?;
        match node.parent {
            None if node.id == 0 && node.is_dir => {}
            Some(parent) if node.id > 0 && parent < node.id => {}
            _ => return Err(invalid("peer tree has an invalid parent relationship")),
        }
        if !self.seen.insert(node.id) {
            return Err(invalid("peer tree contains a duplicate node id"));
        }

        let mut built_children = self.waiting.remove(&node.id).unwrap_or_default();
        if !node.is_dir && !built_children.is_empty() {
            return Err(invalid("peer tree attaches children to a file"));
        }
        if node.is_dir && node.size != 0 {
            return Err(invalid("peer tree directory carries an untrusted size"));
        }
        let size = if node.is_dir {
            built_children
                .iter()
                .try_fold(0u64, |sum, child| sum.checked_add(child.wire.size))
                .ok_or_else(|| invalid("peer tree recursive size overflow"))?
        } else {
            node.size
        };
        let depth = if node.is_dir {
            built_children
                .iter()
                .map(|child| child.depth)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .filter(|depth| *depth <= MAX_WALK_DEPTH)
                .ok_or_else(|| invalid("peer tree exceeds the depth safety limit"))?
        } else {
            1
        };
        let mut children: Vec<_> = built_children.drain(..).map(|child| child.wire).collect();
        // The server emits direct files before completed directories. One
        // rotation retains the explorer's directory-first convention.
        if let Some(first_dir) = children.iter().position(|child| child.is_dir) {
            children.rotate_left(first_dir);
        }
        self.totals.observe(&node)?;
        let wire = crate::agent_proto::WireNode {
            name: node.name,
            size,
            is_dir: node.is_dir,
            children,
        };
        if let Some(parent) = node.parent {
            self.waiting
                .entry(parent)
                .or_default()
                .push(BuiltNode { wire, depth });
        } else {
            self.root = Some(wire);
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        totals: WalkTotals,
        nodes: u64,
    ) -> io::Result<crate::agent_proto::WireNode> {
        if self.totals != totals || nodes != totals.nodes() || nodes != self.seen.len() as u64 {
            return Err(invalid("peer tree completion totals do not match"));
        }
        if !self.waiting.is_empty() {
            return Err(invalid("peer tree contains orphaned children"));
        }
        self.root
            .ok_or_else(|| invalid("peer tree root is missing"))
    }
}

pub(super) fn validate_name(name: &str, root: bool) -> io::Result<()> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || (name.contains('/') || name.contains('\\') || name.contains('\0'))
            && !(root && name == "/")
        || (!root && matches!(name, "." | ".."))
    {
        return Err(invalid("peer tree contains an invalid path segment"));
    }
    Ok(())
}

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn count_overflow() -> io::Error {
    invalid("peer tree counters overflow")
}
