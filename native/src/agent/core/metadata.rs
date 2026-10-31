use crate::agent_proto::WireMeta;
use crate::vfs::VfsMeta;

/// The entry of a server's listing. `dot_hidden`: names with a leading dot
/// count as hidden, as SFTP lists them (an SSH agent; the background service
/// passes its backend's own entries and sends no hidden attribute).
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
