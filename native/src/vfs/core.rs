use std::io::{self, Read, Write};
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
    /// its listing - Google Drive `md5Checksum`, Nextcloud/ownCloud
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

    /// Copy within THIS backend. The default streams read->write; `LocalBackend`
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
    /// the "write temp then rename" safe-copy pattern correct. Default false -
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

    /// Does the backend expose a free content hash (MD5) in its listings -
    /// Google Drive `md5Checksum`, Nextcloud/ownCloud `oc:checksums`? When true,
    /// sync can compare by content WITHOUT downloading this side.
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
    /// to the client-side walk. `on_progress(files, bytes)` is called as the walk
    /// streams progress and returns `false` to request cancellation. Only the
    /// agent backend overrides this; blocking (run it off the UI thread).
    fn walk_tree(
        &self,
        _root: &str,
        _on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> Option<crate::agent_proto::WireNode> {
        None
    }

    /// Can this backend transfer an entire subtree in ONE session (the SSH
    /// agent's `GetTree`/`PutTree`)? When true, folder download/upload skips the
    /// per-file round-trips.
    fn supports_bulk_tree(&self) -> bool {
        false
    }

    /// Download the remote subtree rooted at `root` into local `dst` (the
    /// contents of `root` land directly under `dst`), in one streamed session.
    /// Returns the number of files written. Only the agent overrides this.
    fn get_tree(&self, root: &str, dst: &std::path::Path) -> VfsResult<u64> {
        let _ = (root, dst);
        Err(io::Error::new(io::ErrorKind::Unsupported, "bulk tree transfer not supported"))
    }

    /// Upload the local subtree `src` into remote `root` (the contents of `src`
    /// land directly under `root`), in one streamed session. Returns the number
    /// of files sent. Only the agent overrides this.
    fn put_tree(&self, src: &std::path::Path, root: &str) -> VfsResult<u64> {
        let _ = (src, root);
        Err(io::Error::new(io::ErrorKind::Unsupported, "bulk tree transfer not supported"))
    }

    /// Can this backend run a recursive search SERVER-SIDE (the agent's
    /// `Search`)? When true, a recursive name search on a remote streams only
    /// the matches back instead of enumerating the whole tree client-side.
    fn supports_search(&self) -> bool {
        false
    }

    /// Recursively search under `root` server-side, streaming each match into
    /// `tx` (paths RELATIVE to `root`). Returns true if the backend ran the
    /// search (false = unsupported -> caller does a client-side walk). `cancel`
    /// aborts the stream. Only the agent overrides this.
    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: crossbeam_channel::Sender<SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> bool {
        let _ = (root, spec, tx, cancel);
        false
    }

    /// Can this backend produce the SYNC SIGNATURE (size+mtime, and MD5 on
    /// demand) in one SERVER-SIDE walk (the agent's `WalkHashed`)? When true,
    /// `bisync::walk_files` gets the whole tree - including content hashes -
    /// without downloading a single file.
    fn supports_walk_hashed(&self) -> bool {
        false
    }

    /// Walk `root` server-side, streaming a `HashHit` per entry (rel path) into
    /// `tx`; computes MD5 per file when `want_hash`. Returns true if the backend
    /// ran it (false = unsupported -> caller does the client-side walk). `cancel`
    /// aborts. Only the agent overrides this.
    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: crossbeam_channel::Sender<HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> bool {
        let _ = (root, want_hash, tx, cancel);
        false
    }
}

/// One server-side search match (path relative to the search root).
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

/// One entry of a server-side signature walk (path relative to the walk root).
/// `md5` is the hex content hash, present only for files when hashing was asked.
#[derive(Clone, Debug)]
pub struct HashHit {
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
    pub md5: Option<String>,
}

pub type BackendHandle = Arc<dyn Backend>;
