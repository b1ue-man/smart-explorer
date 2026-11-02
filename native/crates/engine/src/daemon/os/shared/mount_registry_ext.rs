//! `registry-2.json` (DRV-M3, strand S2 C4 of plan 2026-10-17): drives of
//! the kinds that version 0.5.182 cannot read (`Devices`, `Vault`), in the
//! form `{version: 1, mounts: [MountConfig]}` and under the rules of
//! `registry.json`: a plain file of at most 1 MiB, at most 256 drives
//! together with it.
//!
//! A version that reads this file may meet a kind that only a later version
//! knows. Such an entry stays a JSON value: it is never started, and it is
//! written back in place, so a rollback does not destroy the drives of the
//! newer version. Every other unreadable entry makes the file invalid, as in
//! `registry.json`.
use serde::Deserialize;
use serde_json::Value;
use std::io;
use std::path::Path;

use crate::mount::MountConfig;

use super::{invalid_data, read_bounded, write_file, MAX_MOUNTS};

pub(super) const SECOND_FILE: &str = "registry-2.json";
const SECOND_VERSION: u32 = 1;
/// The kinds this version reads, as serde names the variants of
/// `MountSource`.
const KNOWN_SOURCES: [&str; 5] = ["SavedRemote", "GoogleDrive", "Peer", "Devices", "Vault"];

#[derive(Default)]
pub(super) struct SecondFile {
    pub(super) mounts: Vec<MountConfig>,
    /// Entries of kinds a later version added, in file order.
    pub(super) foreign: Vec<Value>,
}

impl SecondFile {
    pub(super) fn len(&self) -> usize {
        self.mounts.len() + self.foreign.len()
    }
}

#[derive(Deserialize)]
struct Wire {
    version: u32,
    mounts: Vec<Value>,
}

pub(super) fn read(directory: &Path) -> io::Result<SecondFile> {
    let Some(body) = read_bounded(&directory.join(SECOND_FILE))? else {
        return Ok(SecondFile::default());
    };
    let wire: Wire = serde_json::from_slice(&body)
        .map_err(|error| invalid_data(format!("invalid mount registry-2: {error}")))?;
    if wire.version != SECOND_VERSION {
        return Err(invalid_data("unsupported mount registry-2 version"));
    }
    if wire.mounts.len() > MAX_MOUNTS {
        return Err(invalid_data("mount registry-2 contains too many entries"));
    }
    let mut file = SecondFile::default();
    for value in wire.mounts {
        if later_kind(&value) {
            file.foreign.push(value);
            continue;
        }
        let config: MountConfig = serde_json::from_value(value)
            .map_err(|error| invalid_data(format!("invalid mount registry-2 entry: {error}")))?;
        config.validate()?;
        file.mounts.push(config);
    }
    Ok(file)
}

pub(super) fn write(directory: &Path, file: &mut SecondFile) -> io::Result<()> {
    file.mounts
        .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    write_file(directory, SECOND_FILE, &encode(file)?)
}

/// `{"version":1,"mounts":[…]}`: the drives in the field order of their
/// type (as in `registry.json`), then the entries of later kinds.
fn encode(file: &SecondFile) -> io::Result<Vec<u8>> {
    let mut entries = Vec::with_capacity(file.len());
    for config in &file.mounts {
        entries.push(serde_json::to_string(config).map_err(encode_error)?);
    }
    for value in &file.foreign {
        entries.push(serde_json::to_string(value).map_err(encode_error)?);
    }
    Ok(format!(
        "{{\"version\":{SECOND_VERSION},\"mounts\":[{}]}}",
        entries.join(",")
    )
    .into_bytes())
}

/// An entry whose source names a kind this version does not know.
fn later_kind(value: &Value) -> bool {
    let source = &value["source"];
    let kind = source.as_str().or_else(|| {
        source
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.keys().next())
            .map(String::as_str)
    });
    kind.is_some_and(|kind| !KNOWN_SOURCES.contains(&kind))
}

fn encode_error(error: serde_json::Error) -> io::Error {
    invalid_data(format!("encode mount registry-2: {error}"))
}

#[cfg(test)]
pub(super) fn known_sources() -> &'static [&'static str] {
    &KNOWN_SOURCES
}
