//! Bounded, iterative transfer of the local worker's retained tree.
//! Counts of retained nodes are deliberately separate from scanned file counts.
use super::{Progress, SizeNode};
use serde::{Deserialize, Serialize};
use std::io;

pub(crate) const MAX_NODES: u64 = 12_000_002;
pub(crate) const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 2052;
const MAX_NAME: usize = 32 * 1024;
const RECORD_FIXED: usize = 17;
const CHUNK: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TreeShape {
    pub nodes: u64,
    pub bytes: u64,
}

impl TreeShape {
    pub(crate) fn validate(self) -> io::Result<()> {
        if self.nodes > MAX_NODES
            || self.bytes > MAX_BYTES
            || self.bytes < self.nodes.saturating_mul(RECORD_FIXED as u64)
            || (self.nodes == 0) != (self.bytes == 0)
        {
            return Err(invalid("Ungültige Größe des Analyse-Ergebnisses"));
        }
        Ok(())
    }

    fn record(&mut self, name: &str, depth: usize) -> io::Result<()> {
        if depth > MAX_DEPTH || name.is_empty() || name.len() > MAX_NAME || name.contains('\0') {
            return Err(invalid("Ungültiger Analyse-Baumeintrag"));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or_else(|| invalid("Zu viele Einträge"))?;
        self.bytes = self
            .bytes
            .checked_add((RECORD_FIXED + name.len()) as u64)
            .ok_or_else(|| invalid("Analyse-Ergebnis zu groß"))?;
        self.validate()
    }
}

pub(crate) fn shape(tree: Option<&SizeNode>, progress: &Progress) -> io::Result<TreeShape> {
    let mut result = TreeShape::default();
    let mut stack = Vec::new();
    if let Some(tree) = tree {
        stack.push((std::slice::from_ref(tree).iter(), 0));
    }
    while let Some((children, depth)) = stack.last_mut() {
        progress.check_cancel()?;
        let Some(node) = children.next() else {
            stack.pop();
            continue;
        };
        let depth = *depth;
        result.record(&node.name, depth)?;
        stack.push((node.children.iter(), depth + 1));
    }
    Ok(result)
}

pub(crate) fn encode(
    tree: Option<SizeNode>,
    progress: &Progress,
    mut emit: impl FnMut(Vec<u8>) -> io::Result<()>,
) -> io::Result<()> {
    let mut stack = vec![tree.into_iter().collect::<Vec<_>>().into_iter()];
    let mut batch = Vec::with_capacity(CHUNK);
    while let Some(children) = stack.last_mut() {
        progress.check_cancel()?;
        let Some(mut node) = children.next() else {
            stack.pop();
            continue;
        };
        if batch.len() + RECORD_FIXED + node.name.len() > CHUNK {
            emit(std::mem::replace(&mut batch, Vec::with_capacity(CHUNK)))?;
        }
        batch.extend_from_slice(&(node.name.len() as u32).to_le_bytes());
        batch.extend_from_slice(node.name.as_bytes());
        batch.push(u8::from(node.is_dir));
        batch.extend_from_slice(&node.size.to_le_bytes());
        let count = u32::try_from(node.children.len()).map_err(|_| invalid("Zu viele Kinder"))?;
        batch.extend_from_slice(&count.to_le_bytes());
        stack.push(std::mem::take(&mut node.children).into_iter());
    }
    if !batch.is_empty() {
        emit(batch)?;
    }
    Ok(())
}

struct Pending {
    node: SizeNode,
    remaining: u32,
}

#[derive(Default)]
pub(crate) struct TreeDecoder {
    pending: Vec<Pending>,
    root: Option<SizeNode>,
    observed: TreeShape,
    buffered: Vec<u8>,
    limit: Option<TreeShape>,
}

impl TreeDecoder {
    pub(crate) fn set_shape(&mut self, shape: TreeShape) {
        self.limit = Some(shape);
    }
    pub(crate) fn push(&mut self, bytes: &[u8], progress: &Progress) -> io::Result<()> {
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err(invalid("Ungültiger Analyse-Datenblock"));
        }
        self.buffered.extend_from_slice(bytes);
        let mut consumed = 0;
        loop {
            let bytes = &self.buffered[consumed..];
            if bytes.len() < 4 {
                break;
            }
            let length = u32::from_le_bytes(
                bytes[..4]
                    .try_into()
                    .map_err(|_| invalid("Ungültiger Name"))?,
            ) as usize;
            if length > MAX_NAME {
                return Err(invalid("Ungültiger Name"));
            }
            let record = RECORD_FIXED + length;
            if bytes.len() < record {
                break;
            }
            let mut bytes = &bytes[..record];
            progress.check_cancel()?;
            let length = u32::from_le_bytes(take::<4>(&mut bytes)?) as usize;
            if length > MAX_NAME || bytes.len() < length {
                return Err(invalid("Ungültiger Name"));
            }
            let name =
                std::str::from_utf8(&bytes[..length]).map_err(|_| invalid("Ungültiges UTF-8"))?;
            self.observed.record(name, self.pending.len())?;
            if self.limit.is_some_and(|limit| {
                self.observed.nodes > limit.nodes || self.observed.bytes > limit.bytes
            }) {
                return Err(invalid("Analyse überschreitet die angekündigte Baumgröße"));
            }
            let name = name.to_string().into_boxed_str();
            bytes = &bytes[length..];
            let is_dir = match take::<1>(&mut bytes)?[0] {
                0 => false,
                1 => true,
                _ => return Err(invalid("Ungültiger Eintragstyp")),
            };
            let size = u64::from_le_bytes(take::<8>(&mut bytes)?);
            let remaining = u32::from_le_bytes(take::<4>(&mut bytes)?);
            if (!is_dir && remaining != 0)
                || u64::from(remaining) > MAX_NODES
                || self.root.is_some()
                || self.limit.is_some_and(|limit| {
                    u64::from(remaining) > limit.nodes.saturating_sub(self.observed.nodes)
                })
            {
                return Err(invalid("Ungültige Analyse-Baumstruktur"));
            }
            let node = SizeNode {
                name,
                size,
                is_dir,
                children: Vec::new(),
            };
            self.pending.push(Pending { node, remaining });
            self.complete_nodes()?;
            consumed += record;
        }
        self.buffered.drain(..consumed);
        Ok(())
    }

    fn complete_nodes(&mut self) -> io::Result<()> {
        while self.pending.last().is_some_and(|node| node.remaining == 0) {
            let completed = self
                .pending
                .pop()
                .ok_or_else(|| invalid("Fehlender Knoten"))?
                .node;
            if completed.is_dir {
                let total = completed
                    .children
                    .iter()
                    .try_fold(0u64, |sum, node| sum.checked_add(node.size));
                if total != Some(completed.size) {
                    return Err(invalid("Widersprüchliche Ordnergröße"));
                }
            }
            if let Some(parent) = self.pending.last_mut() {
                parent.remaining -= 1;
                parent.node.children.push(completed);
            } else {
                if !completed.is_dir {
                    return Err(invalid("Analyse-Wurzel ist kein Ordner"));
                }
                self.root = Some(completed);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self, expected: TreeShape) -> io::Result<Option<SizeNode>> {
        expected.validate()?;
        if !self.buffered.is_empty()
            || !self.pending.is_empty()
            || self.observed != expected
            || self.root.is_some() != (expected.nodes > 0)
        {
            return Err(invalid("Unvollständiges Analyse-Ergebnis"));
        }
        Ok(self.root)
    }
}

fn take<const N: usize>(bytes: &mut &[u8]) -> io::Result<[u8; N]> {
    let prefix = bytes
        .get(..N)
        .ok_or_else(|| invalid("Abgeschnittener Analyse-Datenblock"))?;
    let value = prefix.try_into().map_err(|_| invalid("Ungültiges Feld"))?;
    *bytes = &bytes[N..];
    Ok(value)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
