use std::io::{Read, Write};

use super::local_platform;
use super::{Backend, Scheme, VfsMeta, VfsResult};

// Intentionally duplicated from `scanner.rs` (tiny) to keep this module
// self-contained - isolation over DRY, per the staged remote-layer plan.

#[inline]
fn ms_since_unix(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

fn meta_to_vfs(name: String, meta: &std::fs::Metadata) -> VfsMeta {
    let (hidden, system) = local_platform::local_attrs(meta);
    let is_symlink = meta.is_symlink() || local_platform::is_reparse_point(meta);
    let is_dir = meta.is_dir() && !is_symlink;
    VfsMeta {
        name,
        is_dir,
        is_symlink,
        size: if is_dir { 0 } else { meta.len() },
        mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
        btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
        hidden,
        system,
        id: None,
        content_md5: None,
    }
}

/// `std::fs`-backed local disk using the host path adapter at the boundary.
pub struct LocalBackend {
    root: String, // forward-slash, trailing slash trimmed (display only)
}

impl LocalBackend {
    pub fn new(root: &str) -> Self {
        let r = root.trim().replace('\\', "/");
        let r = r.trim_end_matches('/');
        LocalBackend {
            root: if r.is_empty() {
                "/".to_string()
            } else {
                r.to_string()
            },
        }
    }
}

impl Backend for LocalBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Local
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let dir = local_platform::to_os(path);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(meta) = std::fs::symlink_metadata(entry.path()) {
                out.push(meta_to_vfs(name, &meta));
            }
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let p = local_platform::to_os(path);
        let meta = std::fs::symlink_metadata(&p)?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        Ok(meta_to_vfs(name, &meta))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(std::fs::File::open(local_platform::to_os(path))?))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(std::fs::File::create(local_platform::to_os(
            path,
        ))?))
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        std::fs::copy(local_platform::to_os(src), local_platform::to_os(dst))
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        std::fs::rename(local_platform::to_os(src), local_platform::to_os(dst))
    }
    fn rename_overwrites(&self) -> bool {
        true // std::fs::rename atomically replaces an existing destination
    }
    fn is_local(&self) -> bool {
        true // a local disk read to hash a file is cheap (no network)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_file(local_platform::to_os(path))
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_dir(local_platform::to_os(path))
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        std::fs::create_dir_all(local_platform::to_os(path))
    }
}
