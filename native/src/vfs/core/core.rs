use std::io::{self, Read, Write};
use std::sync::Arc;

use super::capabilities::{RootConfinement, StagedWriteCapabilities};

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
    Peer,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Upsert,
    Remove,
}

/// One backend-reported change. `rel` is optional because ID-addressed remotes
/// such as Drive may report a stable file id and parent id; the sync index can
/// resolve that into a relative path from previous state.
#[derive(Clone, Debug)]
pub struct VfsChange {
    pub kind: ChangeKind,
    pub rel: Option<String>,
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub meta: Option<VfsMeta>,
}

#[derive(Clone, Debug, Default)]
pub struct VfsChangeBatch {
    pub changes: Vec<VfsChange>,
    pub new_cursor: Option<String>,
    pub reset: bool,
}

pub type VfsResult<T> = io::Result<T>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteDisposition {
    Recycle,
    Permanent,
    Unsupported,
}

/// One exact, read-only-planned duplicate cleanup target. Backends with stable
/// IDs must populate `id` so applying an earlier safety preflight cannot delete
/// a different same-name object after concurrent changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DedupeCandidate {
    pub path: String,
    pub id: Option<String>,
}

/// The storage interface. One implementation per protocol. `Send + Sync` so a
/// single handle can be shared across rayon workers / scan + copy threads.
pub trait Backend: Send + Sync {
    fn scheme(&self) -> Scheme;

    /// Forward-slash display root (where navigation starts / what the UI shows).
    fn root_display(&self) -> String;

    /// Stable, non-secret identity for persisted side-specific state. Remote
    /// backends should include their account/host endpoint, not only a path, so
    /// two different connections named `/` cannot share a sync baseline.
    fn state_identity(&self) -> String {
        format!("{:?}:{}", self.scheme(), self.root_display())
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>>;
    fn stat(&self, path: &str) -> VfsResult<VfsMeta>;

    /// Check whether `path` exists without treating access, transport, or
    /// parsing failures as absence. Safety-critical overwrite and uniqueness
    /// decisions must use this fallible form.
    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        match self.stat(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Best-effort existence hint retained for non-destructive UI paths.
    /// Mutating code must use `try_exists` so failures cannot look absent.
    fn exists(&self, path: &str) -> bool {
        self.try_exists(path).unwrap_or(false)
    }

    /// Stable backend identity for `path`, when the provider has one. Local
    /// filesystems normally return None; Drive returns the file id.
    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        let _ = path;
        Ok(None)
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>>;
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>>;

    /// Open a new regular file without replacing, truncating, or adopting an
    /// existing namespace entry. Mounted writes use this for their private
    /// upload staging object so a probe/create race cannot target another
    /// actor's file. Backends must use one protocol/OS exclusive-create call.
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "backend has no atomic exclusive-create writer",
        ))
    }

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
        let staged = super::promotion::unique_staging_path(self, dst, "copy")?;
        let result = (|| {
            let mut writer = self.open_write(&staged)?;
            let copied = io::copy(&mut r, &mut writer)?;
            writer.flush()?;
            drop(writer);
            super::promotion::promote_staged_replace(self, &staged, dst)?;
            Ok(copied)
        })();
        if result.is_err() {
            let _ = self.remove_file(&staged);
        }
        result
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()>;

    /// Move `src` to a destination that must not already exist. This is a hard
    /// atomicity contract: an existence probe followed by ordinary rename does
    /// not satisfy it because another writer can create `dst` between calls.
    /// Backends without a protocol/OS no-replace primitive stay unsupported.
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        let _ = (src, dst);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "backend has no atomic no-replace rename",
        ))
    }

    /// Commit a complete staged regular file without exposing partial content
    /// or removing the old file before the replacement is visible. The default
    /// uses only declared atomic primitives. ID-addressed providers may override
    /// this with a verified update of the existing destination identity, but
    /// must leave the namespace unambiguous on every successful return.
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        super::promotion::default_promote_staged(self, staged, destination)
    }

    /// Commit a complete staged regular file only if `destination` is still
    /// absent. This must be one atomic no-replace operation; callers use it
    /// after an absence preflight so a concurrent creator is never replaced.
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        super::promotion::default_promote_staged_no_replace(self, staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()>;
    fn remove_dir(&self, path: &str) -> VfsResult<()>;
    fn mkdir_all(&self, path: &str) -> VfsResult<()>;

    /// Semantics of this backend's `remove_*` methods. Most network protocols
    /// delete permanently; providers such as Drive override this when removal
    /// is recoverable, and read-only backends report Unsupported.
    fn delete_disposition(&self) -> DeleteDisposition {
        DeleteDisposition::Permanent
    }

    /// Directory-walk width. Local = all cores; remote backends return a small
    /// number (a few SSH channels / one control connection).
    fn parallelism(&self) -> usize {
        rayon::current_num_threads()
    }

    /// Does `rename(src, dst)` atomically REPLACE an existing regular-file
    /// `dst`, leaving an observer with either the old or new complete file?
    /// Only then is the "write temp then rename" safe-copy pattern correct.
    /// Default false. Providers such as Google Drive permit duplicate names and
    /// therefore refuse occupied path renames; SFTP/FTP may also fail when the
    /// target exists.
    /// Local filesystems override this to true.
    fn rename_overwrites(&self) -> bool {
        false
    }

    /// Safe staged-write guarantees at `root`. `create` includes atomic
    /// exclusive creation through `open_write_new`. Backends with path-dependent
    /// exports (notably a Share peer) may inspect the requested subtree.
    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities {
            create: false,
            replace: self.rename_overwrites(),
            namespace_replace: self.rename_overwrites(),
        }
    }

    /// Whether every pathname below `root` has proven case-sensitive lookup
    /// semantics. The conservative default is false: protocols such as SFTP
    /// and generic peer transports can target either Unix-like or Windows
    /// storage, and a local filesystem may itself be mounted case-folded.
    ///
    /// Callers may advertise case-sensitive filesystem behavior only when a
    /// backend can make this guarantee for the exact exported root and retain
    /// it across every reconnect or transport fallback for that backend.
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        false
    }

    /// Whether every operation is technically confined to the exact selected
    /// root even if a pathname is exchanged concurrently after validation.
    /// The conservative default requires an explicit trusted-root opt-in.
    fn root_confinement(&self, _root: &str) -> RootConfinement {
        RootConfinement::Unverified
    }

    /// Open a file for reading by its backend-unique `id` when known (so the
    /// caller can target one specific item among duplicate names). Default
    /// ignores the id and opens by path; Google Drive overrides this.
    fn open_read_id(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> VfsResult<Box<dyn std::io::Read + Send>> {
        let _ = id;
        self.open_read(path)
    }

    /// Delete a file by its backend-unique `id` when known (targets one specific
    /// item among duplicate names). Default ignores the id and deletes by path.
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        let _ = id;
        self.remove_file(path)
    }

    /// Plan the exact duplicate objects a mirror cleanup would remove, without
    /// mutating anything. The orchestration layer uses this count in its delete
    /// safety guard before any copy or delete begins.
    fn plan_dedupe_recursive(
        &self,
        root: &str,
        keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<DedupeCandidate>> {
        let _ = (root, keep);
        Ok(Vec::new())
    }

    /// Apply a previously preflighted cleanup plan. ID-addressed backends must
    /// delete the exact recorded ID rather than resolving the path again.
    fn apply_dedupe_plan(&self, plan: &[DedupeCandidate]) -> VfsResult<usize> {
        let mut removed = 0usize;
        for candidate in plan {
            if let Err(error) = self.remove_file_id(&candidate.path, candidate.id.as_deref()) {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "duplicate cleanup stopped after {removed}/{} exact removals at {} (id {:?}): {error}",
                        plan.len(), candidate.path, candidate.id
                    ),
                ));
            }
            removed += 1;
        }
        Ok(removed)
    }

    /// Make a mirror destination exact on backends that allow duplicate names
    /// (Google Drive): within `root` (recursively), for any name that has MORE
    /// THAN ONE file, keep just the newest if its relative path passes `keep`,
    /// otherwise remove all copies (an orphaned duplicate name). Singleton files
    /// are never touched (the normal plan handles those). Default no-op (names
    /// are already unique). Returns the count removed.
    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        let plan = self.plan_dedupe_recursive(root, keep)?;
        self.apply_dedupe_plan(&plan)
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

    /// Does this backend support an incremental change feed for the subtree?
    fn supports_changes(&self) -> bool {
        false
    }

    /// Stable identity of a sync root, used to detect that a saved cursor belongs
    /// to the same backend folder before an incremental run is trusted.
    fn change_root_id(&self, root: &str) -> VfsResult<Option<String>> {
        let _ = root;
        Ok(None)
    }

    /// Return the cursor for future changes. Providers with snapshot semantics
    /// should return a token before a bootstrap walk, so changes during that
    /// bootstrap are replayed on the next incremental run.
    fn current_change_cursor(&self, root: &str) -> VfsResult<Option<String>> {
        let _ = root;
        Ok(None)
    }

    /// Return changes since `cursor`. `reset = true` means the cursor is invalid
    /// and the caller must rebuild from a full snapshot.
    fn changes_since(&self, root: &str, cursor: &str) -> VfsResult<VfsChangeBatch> {
        let _ = (root, cursor);
        Ok(VfsChangeBatch {
            reset: true,
            ..Default::default()
        })
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
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        Ok(None)
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
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bulk tree transfer not supported",
        ))
    }

    /// Upload the local subtree `src` into remote `root` (the contents of `src`
    /// land directly under `root`), in one streamed session. Returns the number
    /// of files sent. Only the agent overrides this.
    fn put_tree(&self, src: &std::path::Path, root: &str) -> VfsResult<u64> {
        let _ = (src, root);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bulk tree transfer not supported",
        ))
    }

    /// Can this backend run a recursive search SERVER-SIDE (the agent's
    /// `Search`)? When true, a recursive name search on a remote streams only
    /// the matches back instead of enumerating the whole tree client-side.
    fn supports_search(&self) -> bool {
        false
    }

    /// Recursively search under `root` server-side, streaming each match into
    /// `tx` (paths RELATIVE to `root`). `Ok(true)` means the operation completed;
    /// `Ok(false)` is reserved for an explicitly unsupported capability and may
    /// only be returned before sending any hits. Transport, protocol, remote,
    /// and cancellation failures are errors, never fallback signals.
    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: crossbeam_channel::Sender<SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        let _ = (root, spec, tx, cancel);
        Ok(false)
    }

    /// Can this backend produce the SYNC SIGNATURE (size+mtime, and MD5 on
    /// demand) in one SERVER-SIDE walk (the agent's `WalkHashed`)? When true,
    /// `bisync::walk_files` gets the whole tree - including content hashes -
    /// without downloading a single file.
    fn supports_walk_hashed(&self) -> bool {
        false
    }

    /// Walk `root` server-side, streaming a `HashHit` per entry (rel path) into
    /// `tx`; computes MD5 per file when `want_hash`. `Ok(true)` means the walk
    /// completed; `Ok(false)` is reserved for an explicitly unsupported
    /// capability and may only be returned before sending entries. All failures
    /// after dispatch are errors so callers never merge partial and fallback
    /// snapshots.
    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: crossbeam_channel::Sender<HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        let _ = (root, want_hash, tx, cancel);
        Ok(false)
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
