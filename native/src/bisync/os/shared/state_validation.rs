use std::io;

use rusqlite::types::Type;

use super::state_types::{ItemRecord, PairRecord, Side};
use super::types::{Baseline, Sig};

const MAX_STATE_NODES: usize = 1_000_000;
const MAX_STATE_TEXT_BYTES: usize = 128 * 1024 * 1024;
const MAX_STATE_DEPTH: usize = 512;

pub(super) struct StateBudget {
    nodes: usize,
    text_bytes: usize,
    max_nodes: usize,
    max_text_bytes: usize,
}

impl StateBudget {
    pub(super) fn new() -> Self {
        Self {
            nodes: 0,
            text_bytes: 0,
            max_nodes: MAX_STATE_NODES,
            max_text_bytes: MAX_STATE_TEXT_BYTES,
        }
    }

    pub(super) fn for_pair() -> Self {
        Self {
            nodes: 0,
            text_bytes: 0,
            max_nodes: MAX_STATE_NODES.saturating_mul(2),
            max_text_bytes: MAX_STATE_TEXT_BYTES.saturating_mul(2),
        }
    }

    #[cfg(test)]
    pub(super) fn with_limits(max_nodes: usize, max_text_bytes: usize) -> Self {
        Self {
            nodes: 0,
            text_bytes: 0,
            max_nodes,
            max_text_bytes,
        }
    }

    pub(super) fn record_item(&mut self, item: &ItemRecord) -> rusqlite::Result<()> {
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(item_text_bytes(item));
        if self.nodes > self.max_nodes || self.text_bytes > self.max_text_bytes {
            return Err(invalid(
                0,
                Type::Text,
                "sync state exceeds its bounded collection budget",
            ));
        }
        validate_item(item)
    }

    pub(super) fn ensure_fits(&self, nodes: i64, text_bytes: i64) -> rusqlite::Result<()> {
        let nodes = usize::try_from(nodes).ok();
        let text_bytes = usize::try_from(text_bytes).ok();
        if nodes.is_none_or(|nodes| nodes > self.max_nodes.saturating_sub(self.nodes))
            || text_bytes
                .is_none_or(|bytes| bytes > self.max_text_bytes.saturating_sub(self.text_bytes))
        {
            return Err(invalid(
                0,
                Type::Text,
                "sync state exceeds its bounded collection budget",
            ));
        }
        Ok(())
    }
}

pub(super) fn validate_pair_load_bytes(text_bytes: i64) -> rusqlite::Result<()> {
    if usize::try_from(text_bytes).map_or(true, |bytes| bytes > MAX_STATE_TEXT_BYTES) {
        return Err(invalid(0, Type::Text, "sync pair exceeds its text budget"));
    }
    Ok(())
}

pub(super) fn validate_pair(record: &PairRecord) -> rusqlite::Result<()> {
    let text_bytes = record
        .pair
        .len()
        .saturating_add(record.root_a.len())
        .saturating_add(record.root_b.len())
        .saturating_add(record.mode.len())
        .saturating_add(record.source_cursor.as_deref().map_or(0, str::len))
        .saturating_add(record.root_a_id.as_deref().map_or(0, str::len))
        .saturating_add(record.root_b_id.as_deref().map_or(0, str::len));
    if record.pair.is_empty() || record.mode.is_empty() || text_bytes > MAX_STATE_TEXT_BYTES {
        return Err(invalid(0, Type::Text, "invalid or over-budget sync pair"));
    }
    Ok(())
}

pub(super) fn validate_cursor(cursor: Option<&str>) -> rusqlite::Result<()> {
    if cursor.is_some_and(|cursor| cursor.len() > MAX_STATE_TEXT_BYTES) {
        return Err(invalid(
            0,
            Type::Text,
            "sync cursor exceeds its text budget",
        ));
    }
    Ok(())
}

pub(super) fn parse_side(index: usize, value: &str) -> rusqlite::Result<Side> {
    match value {
        "A" => Ok(Side::A),
        "B" => Ok(Side::B),
        _ => Err(invalid(index, Type::Text, "invalid sync side")),
    }
}

pub(super) fn parse_bool(index: usize, value: i64) -> rusqlite::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid(index, Type::Integer, "invalid persisted boolean")),
    }
}

pub(super) fn parse_sig(
    size_index: usize,
    size: Option<String>,
    mtime: Option<i64>,
    hash: Option<String>,
) -> rusqlite::Result<Option<Sig>> {
    match (size, mtime, hash) {
        (None, None, None) => Ok(None),
        (Some(size), Some(mtime_ms), Some(hash)) => Ok(Some(Sig {
            size: size
                .parse()
                .map_err(|_| invalid(size_index, Type::Text, "invalid sync-state size"))?,
            mtime_ms,
            hash: hash
                .parse()
                .map_err(|_| invalid(size_index + 2, Type::Text, "invalid sync-state hash"))?,
        })),
        _ => Err(invalid(
            size_index,
            Type::Text,
            "partial sync-state signature",
        )),
    }
}

pub(super) fn baseline_from_items(
    a: &std::collections::BTreeMap<String, ItemRecord>,
    b: &std::collections::BTreeMap<String, ItemRecord>,
) -> Baseline {
    let mut rels = std::collections::BTreeSet::new();
    rels.extend(a.keys().cloned());
    rels.extend(b.keys().cloned());
    rels.into_iter()
        .filter_map(|rel| {
            let left = active_sig(a.get(&rel));
            let right = active_sig(b.get(&rel));
            (left.is_some() || right.is_some()).then_some((rel, (left, right)))
        })
        .collect()
}

fn active_sig(item: Option<&ItemRecord>) -> Option<Sig> {
    item.filter(|item| !item.deleted).and_then(|item| item.sig)
}

pub(super) fn validate_item(item: &ItemRecord) -> rusqlite::Result<()> {
    validate_relative_path(1, &item.rel)?;
    if item.is_dir {
        return Err(invalid(
            7,
            Type::Integer,
            "directory stored in file sync state",
        ));
    }
    if item.deleted && item.sig.is_some() {
        return Err(invalid(
            8,
            Type::Integer,
            "deleted sync item has a signature",
        ));
    }
    if !item.deleted && item.sig.is_none() {
        return Err(invalid(
            8,
            Type::Integer,
            "active sync item has no signature",
        ));
    }
    Ok(())
}

pub(super) fn validate_relative_path(index: usize, rel: &str) -> rusqlite::Result<()> {
    let path = crate::agent_proto::ValidatedRelativePath::parse(rel)
        .map_err(|error| invalid(index, Type::Text, error.to_string()))?;
    if path.as_str() != rel || path.as_str().split('/').count() > MAX_STATE_DEPTH {
        return Err(invalid(
            index,
            Type::Text,
            "invalid sync-state relative path",
        ));
    }
    Ok(())
}

fn item_text_bytes(item: &ItemRecord) -> usize {
    item.rel
        .len()
        .saturating_add(item.id.as_deref().map_or(0, str::len))
        .saturating_add(item.parent_id.as_deref().map_or(0, str::len))
        .saturating_add(item.name.as_deref().map_or(0, str::len))
}

fn invalid(index: usize, data_type: Type, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        data_type,
        Box::new(io::Error::new(io::ErrorKind::InvalidData, message.into())),
    )
}
