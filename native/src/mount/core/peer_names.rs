//! A Windows mount view of an exact-name Share namespace. These names never
//! change Share exports, saved locators, or the peer's physical filesystem.
use std::collections::{HashMap, HashSet};
use std::io;

use sha2::{Digest, Sha256};

use crate::vfs::VfsMeta;

use super::windows_case::windows_ordinal_key;

const MARKER: &str = " [se-";
const HASH_LENGTH: usize = 64;
const MAX_EXTENSION: usize = 32;

pub(crate) fn project_peer_listing(mut entries: Vec<VfsMeta>) -> io::Result<Vec<VfsMeta>> {
    let mut counts = HashMap::<String, usize>::new();
    let mut literal = HashSet::new();
    for entry in entries.iter().filter(|entry| eligible(&entry.name)) {
        if !literal.insert(entry.name.as_str()) {
            return Err(collision());
        }
        *counts.entry(windows_ordinal_key(&entry.name)).or_default() += 1;
    }
    let mut projected = HashSet::new();
    for entry in entries.iter_mut().filter(|entry| eligible(&entry.name)) {
        if is_peer_alias(&entry.name) || counts[&windows_ordinal_key(&entry.name)] > 1 {
            entry.name = peer_alias(&entry.name);
        }
        if !projected.insert(windows_ordinal_key(&entry.name)) {
            return Err(collision());
        }
    }
    Ok(entries)
}

/// Aliases remain bound to the same exact spelling even if a colliding sibling
/// disappears. Raw alias lookalikes are escaped on listing and cannot take over
/// an existing alias. Missing aliases are never writable destination names.
pub(crate) fn resolve_peer_child(
    entries: Vec<VfsMeta>,
    requested: &str,
) -> io::Result<Option<(VfsMeta, Option<String>)>> {
    let alias = is_peer_alias(requested);
    let key = windows_ordinal_key(requested);
    let mut matched = None;
    for metadata in entries.into_iter().filter(|entry| eligible(&entry.name)) {
        let display = if alias {
            peer_alias(&metadata.name)
        } else {
            if is_peer_alias(&metadata.name) {
                continue;
            }
            metadata.name.clone()
        };
        if windows_ordinal_key(&display) == key
            && matched.replace((metadata, alias.then_some(display))).is_some()
        {
            return Err(collision());
        }
    }
    Ok(matched)
}

pub(crate) fn is_peer_alias(name: &str) -> bool {
    // Mount-owned staging/backup siblings are literal temporary names even
    // when derived from an alias. They are hidden by the existing mount rule.
    if super::metadata_loading::is_reserved_mount_sibling(name) { return false; }
    let folded = windows_ordinal_key(name);
    let Some((_, tail)) = folded.rsplit_once(" [SE-") else { return false };
    let Some((hash, extension)) = tail.split_once(']') else { return false };
    hash.len() == HASH_LENGTH && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && (extension.is_empty() || extension.strip_prefix('.').is_some_and(|extension| {
            !extension.is_empty() && !extension.contains('.')
                && extension.encode_utf16().count() <= MAX_EXTENSION
        }))
}

fn peer_alias(name: &str) -> String {
    let digest = Sha256::digest(name.as_bytes());
    let hash: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let (stem, extension) = name.rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty()
            && extension.encode_utf16().count() <= MAX_EXTENSION)
        .map(|(stem, extension)| (stem, format!(".{extension}")))
        .unwrap_or((name, String::new()));
    let suffix = format!("{MARKER}{hash}]{extension}");
    let budget = 255 - suffix.encode_utf16().count();
    let mut used = 0;
    let stem: String = stem.chars().take_while(|character| {
        used += character.len_utf16();
        used <= budget
    }).collect();
    format!("{stem}{suffix}")
}

fn eligible(name: &str) -> bool {
    super::path::validate_windows_component(name).is_ok()
        && !super::metadata_loading::is_reserved_mount_sibling(name)
}

fn collision() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "peer child names cannot be uniquely projected")
}

#[cfg(test)]
#[path = "peer_names_task_tests.rs"]
mod task_tests;
