//! Virtual filesystem layer — the single, standardized interface Smart Explorer
//! talks to, on top of which every storage backend is built: local disk today,
//! SFTP / FTP / network drives next, cloud later. See
//! `docs/REMOTE_LAYER_PLAN.md`.
//!
//! Design (verified):
//!  * **The trait is BLOCKING.** The whole app is synchronous (rayon +
//!    `std::thread` + crossbeam). Remote backends own a private runtime and
//!    `block_on` internally, so scanner / copy / UI never see async.
//!  * **Paths are FORWARD-SLASH strings.** The app already stores paths that
//!    way; each backend converts to its own convention at the boundary.
//!  * **Self-contained.** This module adds no edits to the hot local scan/copy
//!    loops. `LocalBackend` mirrors today's `std::fs` behavior so the remote
//!    scan/copy paths added with the SFTP/FTP backends (and any later
//!    unification) can route through ONE interface without putting a vtable in
//!    the hot local walk. The local fast path stays exactly as it is.
#![allow(dead_code)] // staged interface: wired in by the SFTP/FTP/connect steps.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// Which backend owns a path. A 1-byte `Copy` tag so it can ride on `FileEntry`
/// (added when the first remote backend is wired) without touching the hot
/// local walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Scheme {
    #[default]
    Local,
    Sftp,
    Ftp,
    Webdav,
    GDrive,
}

/// Backend-neutral directory entry / file metadata. Fields a remote backend
/// can't supply (`btime`, `hidden`, `system`) default to `0` / `false`.
#[derive(Clone, Debug)]
pub struct VfsMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
}

pub type VfsResult<T> = io::Result<T>;

/// The storage interface. One implementation per protocol. `Send + Sync` so a
/// single handle can be shared across rayon workers / scan + copy threads.
pub trait Backend: Send + Sync {
    fn scheme(&self) -> Scheme;

    /// Forward-slash display root (where navigation starts / what the UI shows).
    fn root_display(&self) -> String;

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>>;
    fn stat(&self, path: &str) -> VfsResult<VfsMeta>;
    fn exists(&self, path: &str) -> bool {
        self.stat(path).is_ok()
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>>;
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>>;

    /// Copy within THIS backend. The default streams read→write; `LocalBackend`
    /// overrides with `std::fs::copy`. Cross-backend copies are the caller's job
    /// (read from src backend, write to dst backend).
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let mut r = self.open_read(src)?;
        let mut w = self.open_write(dst)?;
        let n = io::copy(&mut r, &mut w)?;
        Ok(n)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()>;
    fn remove_file(&self, path: &str) -> VfsResult<()>;
    fn remove_dir(&self, path: &str) -> VfsResult<()>;
    fn mkdir_all(&self, path: &str) -> VfsResult<()>;

    /// Directory-walk width. Local = all cores; remote backends return a small
    /// number (a few SSH channels / one control connection).
    fn parallelism(&self) -> usize {
        rayon::current_num_threads()
    }
}

pub type BackendHandle = Arc<dyn Backend>;

// ── helpers ────────────────────────────────────────────────────────────────
// Intentionally duplicated from `scanner.rs` (tiny) to keep this module
// self-contained — isolation over DRY, per the staged remote-layer plan.

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
    }
}

/// Forward-slash path → OS path (no-op separator-wise on Unix).
fn to_os(path: &str) -> PathBuf {
    // A bare drive letter ("C:") is *drive-relative* on Windows — it means the
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

// ── LocalBackend ───────────────────────────────────────────────────────────

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

// ── dispatch ───────────────────────────────────────────────────────────────

/// Build the backend for a root string. Remote schemes are recognized by URL
/// prefix; everything else (drive paths, `\\server\share` UNC, mapped drives)
/// is local. The SFTP/FTP arms are filled in by their respective steps so that
/// adding a protocol never touches the callers.
pub fn backend_for(root: &str) -> io::Result<BackendHandle> {
    let r = root.trim();
    let lower = r.to_ascii_lowercase();
    if lower.starts_with("sftp://") {
        Ok(Arc::new(crate::sftp::backend_from_url(r)?))
    } else if lower.starts_with("ftp://") || lower.starts_with("ftps://") {
        Ok(Arc::new(crate::ftp::backend_from_url(r)?))
    } else {
        Ok(Arc::new(LocalBackend::new(r)))
    }
}

/// Whether a root string is served by a remote (non-local) backend. Lets the
/// app pick the remote scan path and disable the inotify watcher for remote
/// roots without constructing a backend.
pub fn is_remote_root(root: &str) -> bool {
    let lower = root.trim().to_ascii_lowercase();
    lower.starts_with("sftp://") || lower.starts_with("ftp://") || lower.starts_with("ftps://")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("vfs_test_{}_{}_{}", tag, std::process::id(), nanos));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn fwd(p: &Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn local_list_and_stat() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        let be = LocalBackend::new(&fwd(&dir));
        assert_eq!(be.scheme(), Scheme::Local);
        assert_eq!(be.root_display(), fwd(&dir));

        let mut entries = be.list_dir(&fwd(&dir)).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(entries.len(), 2);
        let a = entries.iter().find(|e| e.name == "a.txt").unwrap();
        assert!(!a.is_dir && a.size == 5);
        assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

        let m = be.stat(&format!("{}/a.txt", fwd(&dir))).unwrap();
        assert_eq!(m.name, "a.txt");
        assert_eq!(m.size, 5);
        assert!(be.exists(&format!("{}/a.txt", fwd(&dir))));
        assert!(!be.exists(&format!("{}/nope", fwd(&dir))));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn local_read_write_copy_rename_remove() {
        let dir = temp_dir("rw");
        let be = LocalBackend::new(&fwd(&dir));
        let nested = format!("{}/x/y", fwd(&dir));
        be.mkdir_all(&nested).unwrap();
        let src = format!("{}/src.bin", fwd(&dir));
        be.open_write(&src).unwrap().write_all(b"0123456789").unwrap();

        let dst = format!("{}/copied.bin", nested);
        assert_eq!(be.copy_file(&src, &dst).unwrap(), 10);
        let mut buf = String::new();
        be.open_read(&dst).unwrap().read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "0123456789");

        let renamed = format!("{}/renamed.bin", nested);
        be.rename(&dst, &renamed).unwrap();
        assert!(!be.exists(&dst) && be.exists(&renamed));

        be.remove_file(&renamed).unwrap();
        be.remove_dir(&nested).unwrap();
        assert!(!be.exists(&nested));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copy_file_default_impl_streams() {
        // Exercise the trait's default streaming copy_file (not LocalBackend's
        // override) so the remote backends' inherited path is covered.
        struct Streamed(LocalBackend);
        impl Backend for Streamed {
            fn scheme(&self) -> Scheme {
                self.0.scheme()
            }
            fn root_display(&self) -> String {
                self.0.root_display()
            }
            fn list_dir(&self, p: &str) -> VfsResult<Vec<VfsMeta>> {
                self.0.list_dir(p)
            }
            fn stat(&self, p: &str) -> VfsResult<VfsMeta> {
                self.0.stat(p)
            }
            fn open_read(&self, p: &str) -> VfsResult<Box<dyn Read + Send>> {
                self.0.open_read(p)
            }
            fn open_write(&self, p: &str) -> VfsResult<Box<dyn Write + Send>> {
                self.0.open_write(p)
            }
            fn rename(&self, s: &str, d: &str) -> VfsResult<()> {
                self.0.rename(s, d)
            }
            fn remove_file(&self, p: &str) -> VfsResult<()> {
                self.0.remove_file(p)
            }
            fn remove_dir(&self, p: &str) -> VfsResult<()> {
                self.0.remove_dir(p)
            }
            fn mkdir_all(&self, p: &str) -> VfsResult<()> {
                self.0.mkdir_all(p)
            }
        }
        let dir = temp_dir("stream");
        let be = Streamed(LocalBackend::new(&fwd(&dir)));
        let src = format!("{}/s", fwd(&dir));
        be.open_write(&src).unwrap().write_all(b"abcdef").unwrap();
        let dst = format!("{}/d", fwd(&dir));
        assert_eq!(be.copy_file(&src, &dst).unwrap(), 6);
        let mut buf = Vec::new();
        be.open_read(&dst).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"abcdef");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dispatch_and_remote_detection() {
        assert_eq!(backend_for("/tmp").unwrap().scheme(), Scheme::Local);
        assert_eq!(backend_for(r"C:\Users").unwrap().scheme(), Scheme::Local);
        assert_eq!(backend_for(r"\\server\share").unwrap().scheme(), Scheme::Local);
        assert!(backend_for("sftp://h/p").is_err());
        assert!(backend_for("ftp://h/p").is_err());
        assert!(backend_for("ftps://h/p").is_err());

        assert!(!is_remote_root(r"C:\Users"));
        assert!(!is_remote_root(r"\\server\share"));
        assert!(is_remote_root("sftp://h/p"));
        assert!(is_remote_root("FTP://H/P"));
    }
}
