use crate::vfs::VfsMeta;

pub(super) fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

pub(super) fn to_vfs(name: String, meta: &russh_sftp::protocol::FileAttributes) -> VfsMeta {
    let ft = meta.file_type();
    VfsMeta {
        is_dir: ft.is_dir(),
        is_symlink: ft.is_symlink(),
        size: meta.size.unwrap_or(0),
        // SFTP mtime is unix seconds; no btime / hidden / system attrs.
        mtime_ms: meta.mtime.map(|s| s as i64 * 1000).unwrap_or(0),
        btime_ms: 0,
        hidden: name.starts_with('.'),
        system: false,
        name,
        id: None,
        content_md5: None,
    }
}
