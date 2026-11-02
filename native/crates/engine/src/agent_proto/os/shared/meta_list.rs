//! `meta-list-v1` on the agent's own filesystem: `ListDirMeta` and
//! `StatMeta` answer what `ListDir` and `Stat` answer, plus the listing
//! attributes the agent's walk carries (`walk_hashed.rs`): names with a
//! leading dot are hidden, as SFTP lists them, and the creation time where
//! the file system keeps one. `ListMeta` is the tolerant listing
//! (`ListTolerant`) with the same attributes. Every reply is `HashPart`s
//! whose entries carry the name as `rel`, then `End`.
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::fs::{list_local, stat_local, systemtime_ms};
use super::ops_types::omission;
use super::session::{emit, Sink};
use super::{emit_meta_listing_parts, Frame, WireHashEntry, WireHashMeta, WireMeta, WireOmission};

/// The entry of a `WireMeta` with the attributes of `path`.
fn attributed(meta: WireMeta, path: &Path) -> WireHashEntry {
    let btime_ms = std::fs::symlink_metadata(path)
        .ok()
        .and_then(|metadata| metadata.created().ok())
        .map(systemtime_ms)
        .unwrap_or(0);
    WireHashEntry {
        rel: meta.name.clone(),
        is_dir: meta.is_dir,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        digest: meta.content_md5,
        meta: Some(WireHashMeta {
            hidden: meta.name.starts_with('.'),
            system: false,
            btime_ms,
            is_symlink: meta.is_symlink,
            special: meta.special,
        }),
    }
}

fn send(
    sink: &Sink,
    id: u64,
    entries: Vec<WireHashEntry>,
    omitted: Vec<WireOmission>,
) -> io::Result<()> {
    emit_meta_listing_parts(entries, omitted, |part| emit(sink, id, &part))?;
    emit(sink, id, &Frame::End)
}

/// `ListDirMeta`: the entries of `ListDir` with their attributes.
pub fn handle_list_dir_meta(sink: &Sink, id: u64, path: &str) -> io::Result<()> {
    let base = Path::new(path);
    let entries = list_local(path)?
        .into_iter()
        .map(|meta| {
            let child = base.join(&meta.name);
            attributed(meta, &child)
        })
        .collect();
    send(sink, id, entries, Vec::new())
}

/// `StatMeta`: the entry of `Stat` with its attributes.
pub fn handle_stat_meta(sink: &Sink, id: u64, path: &str) -> io::Result<()> {
    let meta = stat_local(path)?;
    send(
        sink,
        id,
        vec![attributed(meta, Path::new(path))],
        Vec::new(),
    )
}

/// `ListMeta`: the tolerant listing with attributes; only a failure of the
/// enumeration itself fails it.
pub fn handle_list_meta(
    sink: &Sink,
    id: u64,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut entries = Vec::new();
    let mut omitted = Vec::new();
    for entry in std::fs::read_dir(path)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "agent listing canceled",
            ));
        }
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(raw) => {
                omitted.push(WireOmission {
                    rel: raw.to_string_lossy().into_owned(),
                    reason: omission::UNREPRESENTABLE,
                    detail: "Name ist kein gültiges UTF-8".into(),
                });
                continue;
            }
        };
        let child = entry.path();
        match std::fs::symlink_metadata(&child) {
            Ok(metadata) => {
                let (link, special) = super::local_platform::metadata_class(&child, &metadata);
                let meta = WireMeta {
                    name,
                    is_dir: metadata.is_dir() && !link && !special,
                    is_symlink: link,
                    special,
                    size: if special { 0 } else { metadata.len() },
                    mtime_ms: metadata.modified().ok().map(systemtime_ms).unwrap_or(0),
                    content_md5: None,
                };
                entries.push(attributed(meta, &child));
            }
            Err(error) => omitted.push(WireOmission {
                rel: name,
                reason: if error.kind() == io::ErrorKind::NotFound {
                    omission::VANISHED
                } else {
                    omission::UNREADABLE
                },
                detail: error.to_string(),
            }),
        }
    }
    send(sink, id, entries, omitted)
}
