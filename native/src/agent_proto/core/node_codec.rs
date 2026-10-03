use super::codec::Reader;
use super::types::WireNode;
use std::io;

const MAX_TREE_NODES: u64 = 1_000_000;
const MAX_TREE_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 512;

pub const TREE_BUDGET_ERROR: &str = "SE_TREE_DECODE_BUDGET";

/// Offered retention and memory limit of one tree request. This type stays
/// independent of the app (the standalone agent shares the codec).
#[derive(Clone, Copy, Debug)]
pub struct TreeDecodeBudget {
    nodes: u64,
    text_bytes: u64,
    frame_bytes: usize,
}

impl TreeDecodeBudget {
    pub fn new(nodes: u64, memory_bytes: u64) -> Self {
        let reserve = 4 * (2 * std::mem::size_of::<WireNode>() as u64 + 17);
        Self {
            nodes: nodes
                .clamp(1, MAX_TREE_NODES)
                .min((memory_bytes / reserve).max(1)),
            text_bytes: (memory_bytes / 4).min(MAX_TREE_TEXT_BYTES),
            frame_bytes: (memory_bytes / 2).min(super::codec::MAX_FRAME as u64) as usize,
        }
    }

    pub(crate) fn frame_bytes(self) -> usize {
        self.frame_bytes
    }
}

fn budget_error() -> io::Error {
    invalid(TREE_BUDGET_ERROR)
}

struct PendingNode {
    name: String,
    size: u64,
    is_dir: bool,
    remaining: u32,
    children: Vec<WireNode>,
}

impl PendingNode {
    fn finish(self) -> WireNode {
        WireNode {
            name: self.name,
            size: self.size,
            is_dir: self.is_dir,
            children: self.children,
        }
    }
}

pub(super) fn decode_node(reader: &mut Reader<'_>) -> io::Result<WireNode> {
    decode_node_with_budget(reader, None)
}

pub(super) fn decode_node_with_budget(
    reader: &mut Reader<'_>,
    budget: Option<TreeDecodeBudget>,
) -> io::Result<WireNode> {
    let mut nodes = 0u64;
    let mut text_bytes = 0u64;
    let root = read_pending(reader, 0, &mut nodes, &mut text_bytes, budget)?;
    if root.remaining == 0 {
        return Ok(root.finish());
    }
    let mut stack = vec![root];
    loop {
        if stack.last().is_some_and(|node| node.remaining == 0) {
            let completed = stack
                .pop()
                .ok_or_else(|| invalid("wire tree decode stack became empty"))?
                .finish();
            if let Some(parent) = stack.last_mut() {
                parent.remaining -= 1;
                parent.children.push(completed);
                continue;
            }
            return Ok(completed);
        }
        let depth = stack.len();
        let child = read_pending(reader, depth, &mut nodes, &mut text_bytes, budget)?;
        if child.remaining == 0 {
            let parent = stack
                .last_mut()
                .ok_or_else(|| invalid("wire tree child has no parent"))?;
            parent.remaining -= 1;
            parent.children.push(child.finish());
        } else {
            stack.push(child);
        }
    }
}

pub(super) fn validate_node(root: &WireNode) -> io::Result<()> {
    let mut nodes = 0u64;
    let mut text_bytes = 0u64;
    let mut stack = vec![(root, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        record_budget(depth, node.name.len() as u64, &mut nodes, &mut text_bytes)?;
        for child in node.children.iter().rev() {
            stack.push((child, depth.saturating_add(1)));
        }
    }
    Ok(())
}

fn read_pending(
    reader: &mut Reader<'_>,
    depth: usize,
    nodes: &mut u64,
    text_bytes: &mut u64,
    budget: Option<TreeDecodeBudget>,
) -> io::Result<PendingNode> {
    if let Some(limit) = budget {
        if depth > MAX_TREE_DEPTH || *nodes >= limit.nodes {
            return Err(budget_error());
        }
    }
    let name = match budget {
        Some(limit) => reader.tree_string(limit.text_bytes.saturating_sub(*text_bytes) as usize)?,
        None => reader.string()?,
    };
    record_budget(depth, name.len() as u64, nodes, text_bytes)?;
    let size = reader.u64()?;
    let is_dir = reader.bool()?;
    let remaining = reader.u32()?;
    if remaining as usize > reader.remaining() / 17 {
        return Err(invalid("wire tree child count exceeds remaining bytes"));
    }
    if budget.is_some_and(|limit| remaining as u64 > limit.nodes.saturating_sub(*nodes)) {
        return Err(budget_error());
    }
    if !is_dir && remaining > 0 {
        return Err(invalid("wire file has children"));
    }
    Ok(PendingNode {
        name,
        size,
        is_dir,
        remaining,
        children: Vec::with_capacity((remaining as usize).min(1024)),
    })
}

fn record_budget(
    depth: usize,
    name_bytes: u64,
    nodes: &mut u64,
    text_bytes: &mut u64,
) -> io::Result<()> {
    if depth > MAX_TREE_DEPTH {
        return Err(invalid("wire tree exceeds its depth limit"));
    }
    *nodes = nodes.saturating_add(1);
    *text_bytes = text_bytes.saturating_add(name_bytes);
    if *nodes > MAX_TREE_NODES || *text_bytes > MAX_TREE_TEXT_BYTES {
        return Err(invalid("wire tree exceeds its bounded decode budget"));
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
