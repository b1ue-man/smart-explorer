use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io;
use std::mem::size_of;

use crate::vfs::{VfsMeta, VfsResult};

pub(super) type ChildKey = fn(&str) -> String;
pub(super) type EntryIndex = HashMap<String, Option<usize>>;

pub(super) fn exact_child_key(name: &str) -> String {
    name.to_string()
}

pub(super) fn build(entries: &[VfsMeta], child_key: ChildKey) -> (EntryIndex, usize) {
    let mut index = HashMap::with_capacity(entries.len());
    for (position, metadata) in entries.iter().enumerate() {
        match index.entry(child_key(&metadata.name)) {
            Entry::Vacant(entry) => {
                entry.insert(Some(position));
            }
            Entry::Occupied(mut entry) => {
                *entry.get_mut() = None;
            }
        }
    }
    let bytes = size_of::<EntryIndex>()
        .saturating_add(
            index.capacity().saturating_mul(
                size_of::<String>()
                    .saturating_add(size_of::<Option<usize>>())
                    .saturating_add(16),
            ),
        )
        .saturating_add(
            index
                .keys()
                .fold(0usize, |total, key| total.saturating_add(key.capacity())),
        );
    (index, bytes)
}

pub(super) fn lookup(
    entries: &[VfsMeta],
    index: &EntryIndex,
    key: &str,
) -> VfsResult<Option<VfsMeta>> {
    match index.get(key) {
        None => Ok(None),
        Some(None) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "backend contains case-colliding child names",
        )),
        Some(Some(position)) => entries.get(*position).cloned().map_or_else(
            || {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "directory cache index is inconsistent",
                ))
            },
            |metadata| Ok(Some(metadata)),
        ),
    }
}
