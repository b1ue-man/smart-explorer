use crate::agent_proto::WireMeta;
use crate::vfs::VfsMeta;

pub(super) fn wire_to_vfs(m: WireMeta) -> VfsMeta {
    VfsMeta {
        name: m.name,
        is_dir: m.is_dir,
        is_symlink: m.is_symlink,
        size: m.size,
        mtime_ms: m.mtime_ms,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        content_md5: None,
    }
}
