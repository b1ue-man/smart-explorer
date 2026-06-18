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
#[derive(Clone, Debug, Default)]
pub struct VfsMeta {
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub btime_ms: i64,
    pub hidden: bool,
    pub system: bool,
    /// Backend-unique id when names alone aren't unique (e.g. Google Drive
    /// keys by file-id and allows duplicate names in one folder). None = the
    /// path/name uniquely identifies the item (local, SFTP, FTP, WebDAV).
    pub id: Option<String>,
    /// Server-provided content MD5 (hex), if the backend exposes one for free in
    /// its listing — Google Drive `md5Checksum`, Nextcloud/ownCloud
    /// `oc:checksums`. Lets checksum-mode compare without downloading the file.
    /// None = not provided (local/SFTP/FTP, Google-Docs/folders).
    pub content_md5: Option<String>,
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

    /// The local filename a download of `path` should be saved as. Defaults to
    /// `name`; backends that transform content on read (e.g. Google Drive
    /// exporting a Doc to .docx) override this to add the right extension.
    fn download_name(&self, _path: &str, name: &str) -> String {
        name.to_string()
    }

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

    /// Does `rename(src, dst)` atomically REPLACE an existing `dst`? Only then is
    /// the "write temp then rename" safe-copy pattern correct. Default false —
    /// e.g. Google Drive allows duplicate names so a rename creates a second file
    /// instead of overwriting; SFTP/FTP renames may fail if the target exists.
    /// Local filesystems override this to true.
    fn rename_overwrites(&self) -> bool {
        false
    }

    /// Open a file for reading by its backend-unique `id` when known (so the
    /// caller can target one specific item among duplicate names). Default
    /// ignores the id and opens by path; Google Drive overrides this.
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn std::io::Read + Send>> {
        let _ = id;
        self.open_read(path)
    }

    /// Delete a file by its backend-unique `id` when known (targets one specific
    /// item among duplicate names). Default ignores the id and deletes by path.
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        let _ = id;
        self.remove_file(path)
    }

    /// Make a mirror destination exact on backends that allow duplicate names
    /// (Google Drive): within `root` (recursively), for any name that has MORE
    /// THAN ONE file, keep just the newest if its relative path passes `keep`,
    /// otherwise remove all copies (an orphaned duplicate name). Singleton files
    /// are never touched (the normal plan handles those). Default no-op (names
    /// are already unique). Returns the count removed.
    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        let _ = (root, keep);
        Ok(0)
    }

    /// Is this a local-filesystem backend? Reading a local file to hash it is
    /// cheap (no network), so sync may hash the local side to compare against a
    /// remote's free native hash.
    fn is_local(&self) -> bool {
        false
    }

    /// Does the backend expose a free content hash (MD5) in its listings — Google
    /// Drive `md5Checksum`, Nextcloud/ownCloud `oc:checksums`? When true, sync can
    /// compare by content WITHOUT downloading this side.
    fn provides_content_hash(&self) -> bool {
        false
    }

    /// Drop any internal directory-listing cache (no-op unless the backend is
    /// wrapped in `CachingBackend`). Called on an explicit refresh.
    fn invalidate_cache(&self) {}

    /// Does this backend compute a whole-tree size walk server-side (the SSH
    /// remote agent)? When true, the analytics scan calls `walk_tree` instead of
    /// the client-side per-dir recursion.
    fn supports_walk_tree(&self) -> bool {
        false
    }

    /// Walk `root` server-side and return the size tree, or `None` to fall back
    /// to the client-side walk. Only the agent backend overrides this; blocking
    /// (run it off the UI thread).
    fn walk_tree(&self, _root: &str) -> Option<crate::agent_proto::WireNode> {
        None
    }
}

pub type BackendHandle = Arc<dyn Backend>;

/// Wraps any backend with a short-TTL **directory-listing cache** so interactive
/// browsing (back/forward, re-visiting a folder, rapid drilling) doesn't re-list
/// over the network every time. Mutating ops invalidate the affected directory;
/// `invalidate_cache()` clears everything (explicit refresh). NOT used by sync —
/// sync re-opens a fresh backend per run and walks each folder once, so a cache
/// would only add staleness with no hit benefit.
pub struct CachingBackend {
    inner: BackendHandle,
    ttl: std::time::Duration,
    cache: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<VfsMeta>)>>,
}

impl CachingBackend {
    pub fn new(inner: BackendHandle) -> Self {
        Self {
            inner,
            ttl: std::time::Duration::from_secs(20),
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn norm(path: &str) -> String {
        let p = path.trim_end_matches('/');
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    }

    fn parent_of(key: &str) -> Option<String> {
        key.rfind('/').map(|i| if i == 0 { "/".to_string() } else { key[..i].to_string() })
    }

    fn invalidate(&self, path: &str) {
        if let Ok(mut c) = self.cache.lock() {
            let key = Self::norm(path);
            c.remove(&key);
            if let Some(parent) = Self::parent_of(&key) {
                c.remove(&parent); // entry added/removed/renamed changes the parent listing
            }
        }
    }
}

impl Backend for CachingBackend {
    fn scheme(&self) -> Scheme {
        self.inner.scheme()
    }
    fn root_display(&self) -> String {
        self.inner.root_display()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let key = Self::norm(path);
        if let Ok(c) = self.cache.lock() {
            if let Some((at, v)) = c.get(&key) {
                if at.elapsed() < self.ttl {
                    return Ok(v.clone());
                }
            }
        }
        let v = self.inner.list_dir(path)?;
        if let Ok(mut c) = self.cache.lock() {
            // Bound memory: a full-tree analytics scan can list thousands of
            // dirs; drop everything once the map gets large rather than grow it
            // unbounded (browsing only needs a small working set).
            if c.len() >= 4096 {
                c.clear();
            }
            c.insert(key, (std::time::Instant::now(), v.clone()));
        }
        Ok(v)
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.inner.stat(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read_id(path, id)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        // A new file may appear in the parent listing once written.
        self.invalidate(path);
        self.inner.open_write(path)
    }
    fn download_name(&self, path: &str, name: &str) -> String {
        self.inner.download_name(path, name)
    }
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        let r = self.inner.copy_file(src, dst);
        self.invalidate(dst);
        r
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let r = self.inner.rename(src, dst);
        self.invalidate(src);
        self.invalidate(dst);
        r
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.remove_file(path);
        self.invalidate(path);
        r
    }
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        let r = self.inner.remove_file_id(path, id);
        self.invalidate(path);
        r
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.remove_dir(path);
        self.invalidate(path);
        r
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let r = self.inner.mkdir_all(path);
        self.invalidate(path);
        r
    }
    fn parallelism(&self) -> usize {
        self.inner.parallelism()
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        let r = self.inner.dedupe_recursive(root, keep);
        self.invalidate_cache(); // a recursive change can touch many folders
        r
    }
    fn is_local(&self) -> bool {
        self.inner.is_local()
    }
    fn provides_content_hash(&self) -> bool {
        self.inner.provides_content_hash()
    }
    fn invalidate_cache(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
    }
}

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
        id: None,
        content_md5: None,
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
    fn caching_backend_serves_and_invalidates() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Counts how many times the inner backend is actually hit.
        struct Counter(AtomicUsize);
        impl Backend for Counter {
            fn scheme(&self) -> Scheme {
                Scheme::Sftp
            }
            fn root_display(&self) -> String {
                "/".into()
            }
            fn list_dir(&self, _p: &str) -> VfsResult<Vec<VfsMeta>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Vec::new())
            }
            fn stat(&self, _p: &str) -> VfsResult<VfsMeta> {
                Err(io::Error::from(io::ErrorKind::NotFound))
            }
            fn open_read(&self, _p: &str) -> VfsResult<Box<dyn Read + Send>> {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            }
            fn open_write(&self, _p: &str) -> VfsResult<Box<dyn Write + Send>> {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            }
            fn rename(&self, _s: &str, _d: &str) -> VfsResult<()> {
                Ok(())
            }
            fn remove_file(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
            fn remove_dir(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
            fn mkdir_all(&self, _p: &str) -> VfsResult<()> {
                Ok(())
            }
        }
        // Keep a typed handle so we can read the inner hit-counter directly
        // (the trait object hides it).
        let typed = Arc::new(Counter(AtomicUsize::new(0)));
        let cb2 = CachingBackend::new(typed.clone() as BackendHandle);
        cb2.list_dir("/x").unwrap();
        cb2.list_dir("/x").unwrap();
        cb2.list_dir("/x/").unwrap(); // trailing slash → same cache key
        assert_eq!(typed.0.load(Ordering::SeqCst), 1, "repeat listings served from cache");
        cb2.invalidate_cache();
        cb2.list_dir("/x").unwrap();
        assert_eq!(typed.0.load(Ordering::SeqCst), 2, "refresh re-listed");
        cb2.remove_dir("/x/sub").unwrap(); // invalidates parent "/x"
        cb2.list_dir("/x").unwrap();
        assert_eq!(typed.0.load(Ordering::SeqCst), 3, "mutation invalidated the dir");
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
