use std::io::{Read, Write};
use std::path::PathBuf;

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

#[cfg(windows)]
fn local_attrs(meta: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let a = meta.file_attributes();
    (
        a & FILE_ATTRIBUTE_HIDDEN != 0,
        a & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}

#[cfg(not(windows))]
fn local_attrs(_meta: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}

fn meta_to_vfs(name: String, meta: &std::fs::Metadata) -> VfsMeta {
    let (hidden, system) = local_attrs(meta);
    let is_dir = meta.is_dir();
    VfsMeta {
        name,
        is_dir,
        is_symlink: meta.is_symlink(),
        size: if is_dir { 0 } else { meta.len() },
        mtime_ms: meta.modified().map(ms_since_unix).unwrap_or(0),
        btime_ms: meta.created().map(ms_since_unix).unwrap_or(0),
        hidden,
        system,
        id: None,
        content_md5: None,
    }
}

/// Forward-slash path -> OS path (no-op separator-wise on Unix).
fn to_os(path: &str) -> PathBuf {
    // A bare drive letter ("C:") is *drive-relative* on Windows - it means the
    // current directory on that drive, so `read_dir("C:")` lists the wrong
    // folder. Normalize it to the drive root ("C:/") so the local backend (and
    // thus the picker, sync, etc.) sees the actual root.
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{}/", path);
        rooted.as_str()
    } else {
        path
    };
    if std::path::MAIN_SEPARATOR == '/' {
        PathBuf::from(path)
    } else {
        PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

/// `std::fs`-backed local disk. Also serves `\\server\share` UNC and mapped
/// drive letters unchanged (those are just local paths to Windows).
pub struct LocalBackend {
    root: String, // forward-slash, trailing slash trimmed (display only)
}

impl LocalBackend {
    pub fn new(root: &str) -> Self {
        let r = root.trim().replace('\\', "/");
        let r = r.trim_end_matches('/');
        LocalBackend {
            root: if r.is_empty() { "/".to_string() } else { r.to_string() },
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
        let dir = to_os(path);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            // entry.metadata() uses the DirEntry's cached metadata on Windows
            // (no extra syscall); fall back for unreadable reparse points /
            // broken symlinks.
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => std::fs::symlink_metadata(entry.path())?,
            };
            out.push(meta_to_vfs(name, &meta));
        }
        Ok(out)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let p = to_os(path);
        let meta = std::fs::symlink_metadata(&p)?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        Ok(meta_to_vfs(name, &meta))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(std::fs::File::open(to_os(path))?))
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(std::fs::File::create(to_os(path))?))
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        std::fs::copy(to_os(src), to_os(dst))
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        std::fs::rename(to_os(src), to_os(dst))
    }
    fn rename_overwrites(&self) -> bool {
        true // std::fs::rename atomically replaces an existing destination
    }
    fn is_local(&self) -> bool {
        true // a local disk read to hash a file is cheap (no network)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_file(to_os(path))
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        std::fs::remove_dir(to_os(path))
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        std::fs::create_dir_all(to_os(path))
    }
}
