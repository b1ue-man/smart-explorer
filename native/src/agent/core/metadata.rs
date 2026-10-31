use crate::agent_proto::{WireHashEntry, WireHashMeta, WireMeta};
use crate::vfs::VfsMeta;

/// The entry of a server's listing. `dot_hidden`: names with a leading dot
/// count as hidden, as SFTP lists them (an SSH agent; the background service
/// passes its backend's own entries and sends no hidden attribute here, only
/// in `ListMeta` replies: `hash_entry_to_vfs`).
pub(crate) fn wire_to_vfs(m: WireMeta, dot_hidden: bool) -> VfsMeta {
    let hidden = dot_hidden && m.name.starts_with('.');
    VfsMeta {
        name: m.name,
        is_dir: m.is_dir,
        is_symlink: m.is_symlink,
        special: m.special,
        size: m.size,
        mtime_ms: m.mtime_ms,
        btime_ms: 0,
        hidden,
        system: false,
        id: None,
        content_md5: m.content_md5,
    }
}

/// The entries of a server's listing (`wire_to_vfs`).
pub(crate) fn wires_to_vfs(entries: Vec<WireMeta>, dot_hidden: bool) -> Vec<VfsMeta> {
    entries
        .into_iter()
        .map(|m| wire_to_vfs(m, dot_hidden))
        .collect()
}

/// The entry a server of this protocol sends for `m`.
pub(crate) fn vfs_to_wire(m: VfsMeta) -> WireMeta {
    WireMeta {
        name: m.name,
        is_dir: m.is_dir,
        is_symlink: m.is_symlink,
        special: m.special,
        size: m.size,
        mtime_ms: m.mtime_ms,
        content_md5: m.content_md5,
    }
}

/// The entry of a `ListMeta` reply for `m` (`list-meta-v1`): the name as
/// `rel`, the provider MD5 as digest and every listing attribute.
pub(crate) fn vfs_to_hash_entry(m: VfsMeta) -> WireHashEntry {
    WireHashEntry {
        rel: m.name,
        is_dir: m.is_dir,
        size: m.size,
        mtime_ms: m.mtime_ms,
        digest: m.content_md5,
        meta: Some(WireHashMeta {
            hidden: m.hidden,
            system: m.system,
            btime_ms: m.btime_ms,
            is_symlink: m.is_symlink,
            special: m.special,
        }),
    }
}

/// The listed entry of a `ListMeta` reply; one without attributes or with a
/// path instead of a name is refused.
pub(crate) fn hash_entry_to_vfs(entry: WireHashEntry) -> std::io::Result<VfsMeta> {
    let invalid = |message: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
    let meta = entry
        .meta
        .ok_or_else(|| invalid("attribute listing entry without attributes"))?;
    if entry.rel.is_empty() || entry.rel.contains('/') {
        return Err(invalid("attribute listing entry is no name"));
    }
    Ok(VfsMeta {
        name: entry.rel,
        is_dir: entry.is_dir,
        is_symlink: meta.is_symlink,
        special: meta.special,
        size: entry.size,
        mtime_ms: entry.mtime_ms,
        btime_ms: meta.btime_ms,
        hidden: meta.hidden,
        system: meta.system,
        id: None,
        content_md5: entry.digest,
    })
}
