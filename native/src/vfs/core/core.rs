use std::io::{Read, Write};
use std::sync::Arc;

use super::capabilities::{MountPathCapabilities, RootConfinement, StagedWriteCapabilities};
use super::extensions::BackendExtensions;

pub use super::batch::{BatchGet, BatchLimits, BatchPut, BatchPutOutcome, BatchSink};
pub use super::congestion::{congestion_error, congestion_of, Congestion};
pub use super::meta::{
    ChangeKind, DedupeCandidate, DeleteDisposition, HashHit, SearchHit, VfsChange, VfsChangeBatch,
    VfsMeta, VfsResult,
};
pub use super::scheme::Scheme;

/// A byte stream a backend hands out for reading.
pub type ReadStream = Box<dyn Read + Send>;
/// A byte sink a backend hands out for writing.
pub type WriteStream = Box<dyn Write + Send>;

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
    /// Filesystem namespace, independent of the connection's starting folder.
    fn namespace_identity(&self) -> String {
        self.state_identity()
    }
    /// Browsing wrappers expose their live backend for sync revalidation.
    fn uncached_backend(&self) -> Option<BackendHandle> {
        None
    }
    /// Optional extensions (tolerant listing, stage times, host-side search,
    /// …); call them through the `vfs::` functions, which add the fallbacks.
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        None
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>>;
    /// Sync observes actual same-name files, without browsing-only aliases.
    fn list_dir_for_sync(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.list_dir(path)
    }
    /// Such providers require complete duplicate observations before mutation.
    fn has_duplicate_file_names(&self) -> bool {
        false
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta>;

    /// Check whether `path` exists without treating access, transport, or
    /// parsing failures as absence. Safety-critical overwrite and uniqueness
    /// decisions must use this fallible form.
    fn try_exists(&self, path: &str) -> VfsResult<bool> {
        super::trait_defaults::try_exists(self, path)
    }

    /// Best-effort existence hint retained for non-destructive UI paths.
    /// Mutating code must use `try_exists` so failures cannot look absent.
    fn exists(&self, path: &str) -> bool {
        self.try_exists(path).unwrap_or(false)
    }

    /// Stable backend identity for `path`, when the provider has one. Local
    /// filesystems normally return None; Drive returns the file id.
    fn item_id(&self, path: &str) -> VfsResult<Option<String>> {
        super::backend_defaults::item_id(path)
    }

    fn open_read(&self, path: &str) -> VfsResult<ReadStream>;
    fn open_write(&self, path: &str) -> VfsResult<WriteStream>;

    /// Open a new regular file without replacing, truncating, or adopting an
    /// existing namespace entry. Mounted writes use this for their private
    /// upload staging object so a probe/create race cannot target another
    /// actor's file. Backends must use one protocol/OS exclusive-create call.
    fn open_write_new(&self, path: &str) -> VfsResult<WriteStream> {
        super::backend_defaults::open_write_new(path)
    }

    /// Private copy stage that updates no existing identity: exclusive creation
    /// by default; ID providers may reserve their own ID and verify an
    /// unambiguous path on flush (not a mounted exclusive-create API). Neither
    /// opening nor failure authorizes path cleanup.
    fn open_write_copy_stage(&self, path: &str) -> VfsResult<WriteStream> {
        self.open_write_new(path)
    }

    /// Publish a copy without replacing another identity (ID providers verify
    /// unique naming and roll back their own ID; no atomic name reservation).
    fn promote_copy_stage(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.promote_staged_no_replace(staged, destination)
    }

    /// Private copy stage of known final length, so spooling providers can
    /// stream; such a writer fails on `flush` unless exactly `size` bytes came.
    fn open_write_copy_stage_sized(&self, path: &str, size: u64) -> VfsResult<WriteStream> {
        super::backend_defaults::open_write_copy_stage_sized(self, path, size)
    }

    /// Stage for a copy whose source stays (paste, drag, download to a
    /// remote): published by `promote_copy_stage` like any stage, but not
    /// forced to stable storage first (Explorer/cp semantics, spec decision
    /// 1). Sync, mounts and replacements keep `open_write_copy_stage_sized`.
    fn open_write_copy_stage_unsynced(&self, path: &str, size: u64) -> VfsResult<WriteStream> {
        self.open_write_copy_stage_sized(path, size)
    }

    /// Server-side copy of `src` (length `size`) into the new private stage
    /// `stage`, published later via `promote_copy_stage`. `Ok(None)` = stream;
    /// `cancel` ends a long copy early (the caller discards the stage).
    fn server_copy_to_stage(
        &self,
        src: &str,
        stage: &str,
        size: u64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<Option<u64>> {
        super::backend_defaults::server_copy_to_stage(src, stage, size, cancel)
    }

    /// New file inside a folder this transfer created itself, for providers
    /// whose objects appear only complete and never replace (Drive): no stage,
    /// no promotion, exactly `size` bytes. `Ok(None)` = use a copy stage.
    fn open_write_fresh(&self, path: &str, size: u64) -> VfsResult<Option<WriteStream>> {
        super::backend_defaults::open_write_fresh(path, size)
    }

    /// Read from byte `offset` to resume an interrupted download; `Ok(None)`
    /// when this backend cannot start mid-file.
    fn open_read_at(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
    ) -> VfsResult<Option<ReadStream>> {
        super::backend_defaults::open_read_at(path, id, offset)
    }

    /// Can this path be read in ranges `(offset, len)`? Default: no.
    fn supports_range_reads(&self, path: &str) -> bool {
        super::backend_defaults::supports_range_reads(path)
    }

    /// Reads exactly `[offset, offset + len)` (fewer only at the end of the
    /// file). `Ok(None)`: not possible, the caller transfers one stream.
    fn open_read_range(
        &self,
        path: &str,
        id: Option<&str>,
        offset: u64,
        len: u64,
    ) -> VfsResult<Option<ReadStream>> {
        super::ranges::default_open_read_range(self, path, id, offset, len)
    }

    /// A stage several writers fill at once, one range each; `Ok(None)`:
    /// not possible. Default: no backend, until it writes at an offset.
    fn open_segmented_stage(
        &self,
        stage: &str,
        size: u64,
    ) -> VfsResult<Option<Box<dyn super::ranges::SegmentedStage>>> {
        super::backend_defaults::open_segmented_stage(stage, size)
    }

    /// A fixed limit of this provider worth showing next to a transfer (for
    /// example a documented write rate).
    fn transfer_hint(&self) -> Option<String> {
        None
    }

    /// Local file name for a download of `path`; exporting providers (Drive
    /// Docs → .docx) add the extension of the exported format.
    fn download_name(&self, _path: &str, name: &str) -> String {
        name.to_string()
    }

    /// Expected read-stream length, including a known empty file; `None` only
    /// for content transformed on read (never "size 0 means unknown").
    fn read_size(&self, _path: &str, metadata_size: u64) -> VfsResult<Option<u64>> {
        Ok(Some(metadata_size))
    }

    /// Copy within this backend (default: spooled read then write); copies
    /// between backends are the caller's job.
    fn copy_file(&self, src: &str, dst: &str) -> VfsResult<u64> {
        super::copy_transfer::copy_file(self, src, dst)
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()>;

    /// Move `src` to a destination that must not already exist. This is a hard
    /// atomicity contract: an existence probe followed by ordinary rename does
    /// not satisfy it because another writer can create `dst` between calls.
    /// Backends without a protocol/OS no-replace primitive stay unsupported.
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        super::backend_defaults::rename_no_replace(src, dst)
    }

    /// Commit a complete staged file without exposing partial content or
    /// removing the old file first; only declared atomic primitives by default.
    /// ID providers may update the destination identity verifiably, leaving the
    /// namespace unambiguous on success.
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        super::promotion::default_promote_staged(self, staged, destination)
    }
    /// Replace the captured object; duplicate-capable providers must bind by ID.
    fn promote_staged_to_id(
        &self,
        staged: &str,
        destination: &str,
        id: Option<&str>,
    ) -> VfsResult<()> {
        super::promotion::promote_to_id(self, staged, destination, id)
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

    /// Create one directory whose parent exists; an existing plain directory
    /// is success. Override where `mkdir_all` costs a round trip per level.
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.mkdir_all(path)
    }

    /// Create one new directory; `AlreadyExists` when the name is taken. The
    /// default probes first (not atomic); protocol backends override it.
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        super::trait_defaults::create_dir_new(self, path)
    }

    /// Remove a copy stage this client created and never published (abort);
    /// `Unsupported` where ownership of the name cannot be proven.
    fn discard_copy_stage(&self, stage: &str) -> VfsResult<()> {
        super::backend_defaults::discard_copy_stage(stage)
    }

    /// Semantics of `remove_*`: permanent for most protocols, recoverable for
    /// providers such as Drive, Unsupported for read-only backends.
    fn delete_disposition(&self) -> DeleteDisposition {
        DeleteDisposition::Permanent
    }

    /// Directory-walk width: all cores locally, small for remote protocols.
    fn parallelism(&self) -> usize {
        rayon::current_num_threads()
    }

    /// Key of the connection (or local volume) serving `path`; users of one
    /// key share one adaptive concurrency controller. Wrappers forward it.
    fn flow_key(&self, path: &str) -> String {
        super::backend_defaults::flow_key(self, path)
    }

    /// Stable key of this connection across process restarts (no pointer
    /// addresses): scheme and namespace identity by default.
    fn flow_identity(&self, path: &str) -> String {
        super::backend_defaults::flow_identity(self, path)
    }

    /// Hard bound of concurrent transfer operations the protocol or peer
    /// imposes on this connection; `None` = adaptive control only.
    fn transfer_ceiling(&self, path: &str) -> Option<usize> {
        super::backend_defaults::transfer_ceiling(path)
    }

    /// Whether a reader and a writer may be open at once; false (FTP) makes
    /// same-backend copies bridge through a local spool.
    fn concurrent_read_write(&self) -> bool {
        true
    }

    /// Does `rename(src, dst)` atomically replace an existing regular file, so
    /// "write temp, then rename" is a safe replace? Default false (Drive keeps
    /// duplicates, SFTP/FTP may refuse); local filesystems return true.
    fn rename_overwrites(&self) -> bool {
        false
    }

    /// Safe staged-write guarantees at `root` (`create` = atomic `open_write_new`);
    /// path-dependent exports (Share peers) may inspect the subtree.
    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        super::capabilities::staged_write_defaults(self)
    }

    /// Fallibly inspect staged writes without replacing an active mount binding.
    /// Stateful backends must override this with a non-acquiring probe.
    fn probe_staged_write_capabilities(&self, root: &str) -> VfsResult<StagedWriteCapabilities> {
        Ok(self.mount_path_capabilities(root)?.staged_write)
    }

    /// Whether every pathname below `root` has proven case-sensitive lookup.
    /// Conservative default false (SFTP/peers may serve Windows storage, local
    /// filesystems may be case-folded); true only when the exact root keeps the
    /// guarantee across every reconnect and transport fallback.
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        false
    }

    /// Whether every operation stays confined to the exact root even if a path
    /// is exchanged after validation; requires an explicit trusted-root opt-in.
    fn root_confinement(&self, _root: &str) -> RootConfinement {
        RootConfinement::Unverified
    }

    /// Mount write and root guarantees as one snapshot (dynamic proxies
    /// override this with one fallible remote probe).
    fn mount_path_capabilities(&self, root: &str) -> VfsResult<MountPathCapabilities> {
        super::capabilities::mount_path_defaults(self, root)
    }

    /// Open by backend-unique `id` when known (one item among duplicate names);
    /// the default opens by path.
    fn open_read_id(&self, path: &str, id: Option<&str>) -> VfsResult<ReadStream> {
        super::backend_defaults::open_read_id(self, path, id)
    }

    /// Delete by backend-unique `id` when known; the default deletes by path.
    fn remove_file_id(&self, path: &str, id: Option<&str>) -> VfsResult<()> {
        super::backend_defaults::remove_file_id(self, path, id)
    }

    /// Plan (without mutating) the exact duplicates a mirror cleanup would
    /// remove; its count feeds the delete safety guard before any change.
    fn plan_dedupe_recursive(
        &self,
        root: &str,
        keep: &dyn Fn(&str) -> bool,
    ) -> VfsResult<Vec<DedupeCandidate>> {
        super::backend_defaults::plan_dedupe_recursive(root, keep)
    }

    /// Apply a previously preflighted cleanup plan. ID-addressed backends must
    /// delete the exact recorded ID rather than resolving the path again.
    fn apply_dedupe_plan(&self, plan: &[DedupeCandidate]) -> VfsResult<usize> {
        super::dedupe::apply(self, plan)
    }

    /// Make a mirror destination exact where duplicate names exist (Drive): for
    /// each name with several files below `root`, keep only the newest if `keep`
    /// accepts its path, else remove all; singletons untouched. Returns removals.
    fn dedupe_recursive(&self, root: &str, keep: &dyn Fn(&str) -> bool) -> VfsResult<usize> {
        super::backend_defaults::dedupe_recursive(self, root, keep)
    }

    /// Local filesystem (hashing a file is cheap, no network)?
    fn is_local(&self) -> bool {
        false
    }

    /// Free content MD5 in listings (Drive `md5Checksum`, Nextcloud checksums)?
    fn provides_content_hash(&self) -> bool {
        false
    }

    /// Does this backend support an incremental change feed for the subtree?
    fn supports_changes(&self) -> bool {
        false
    }

    /// Stable identity of a sync root, so a saved cursor is trusted only there.
    fn change_root_id(&self, root: &str) -> VfsResult<Option<String>> {
        super::backend_defaults::change_root_id(root)
    }

    /// Cursor for future changes, taken before a bootstrap walk so changes
    /// during the walk replay on the next incremental run.
    fn current_change_cursor(&self, root: &str) -> VfsResult<Option<String>> {
        super::backend_defaults::current_change_cursor(root)
    }

    /// Changes since `cursor`; `reset` = invalid cursor, rebuild from a snapshot.
    fn changes_since(&self, root: &str, cursor: &str) -> VfsResult<VfsChangeBatch> {
        super::backend_defaults::changes_since(root, cursor)
    }

    /// Drop any listing cache (explicit refresh; only `CachingBackend` has one).
    fn invalidate_cache(&self) {}

    /// Optional complete remote analytics outcome, including partial results.
    fn scan_storage(
        &self,
        _root: &str,
        _progress: &crate::analytics::Progress,
    ) -> VfsResult<Option<crate::analytics::ScanOutcome>> {
        Ok(None)
    }

    /// Whether a remote agent supports the older tree-only analysis operation.
    fn supports_walk_tree(&self) -> bool {
        false
    }

    /// Legacy server-side tree and measured file/byte progress; false cancels.
    fn walk_tree(
        &self,
        _root: &str,
        _on_progress: &(dyn Fn(u64, u64) -> bool + Sync),
    ) -> VfsResult<Option<crate::agent_proto::WireNode>> {
        Ok(None)
    }

    /// Whole-subtree transfer in one session (the agent's `GetTree`/`PutTree`)?
    fn supports_bulk_tree(&self) -> bool {
        false
    }

    /// Download the contents of `root` into local `dst` in one session;
    /// returns the number of files written.
    fn get_tree(&self, root: &str, dst: &std::path::Path) -> VfsResult<u64> {
        super::backend_defaults::get_tree(root, dst)
    }

    /// Upload the contents of local `src` into `root` in one session; returns
    /// the number of files sent.
    fn put_tree(&self, src: &std::path::Path, root: &str) -> VfsResult<u64> {
        super::backend_defaults::put_tree(src, root)
    }

    /// Batch limits when this backend (and its peer) move many small whole
    /// files in one round trip for paths below `dir`; `None` = unsupported.
    fn batch_limits(&self, dir: &str) -> Option<BatchLimits> {
        super::backend_defaults::batch_limits(dir)
    }

    /// Create every entry as a new file (private stage, no-replace publish,
    /// numbered name when taken) from `data` = the bytes back to back. One
    /// outcome per entry; `Err` = outcome unknown, never retried blindly.
    fn put_batch(
        &self,
        entries: &[BatchPut],
        data: &mut dyn Read,
    ) -> VfsResult<Vec<BatchPutOutcome>> {
        super::backend_defaults::put_batch(entries, data)
    }

    /// Read every item and hand it to `sink` in request order.
    fn get_batch(&self, items: &[BatchGet], sink: &mut dyn BatchSink) -> VfsResult<()> {
        super::backend_defaults::get_batch(items, sink)
    }

    /// Recursive search on the server (the agent's `Search`), streaming matches?
    fn supports_search(&self) -> bool {
        false
    }

    /// Search below `root` on the server, streaming matches (paths relative to
    /// `root`) into `tx`. `Ok(false)` only for "unsupported" before any hit;
    /// every failure (transport, remote, cancel) is an error, never a fallback.
    fn search(
        &self,
        root: &str,
        spec: &crate::agent_proto::SearchSpec,
        tx: crossbeam_channel::Sender<SearchHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        super::backend_defaults::search(root, spec, tx, cancel)
    }

    /// Sync signatures (size, mtime, MD5 on demand) in one server-side walk
    /// (the agent's `WalkHashed`), without downloading files?
    fn supports_walk_hashed(&self) -> bool {
        false
    }

    /// Walk `root` on the server, streaming a `HashHit` per entry into `tx`
    /// (MD5 when `want_hash`). `Ok(false)` only for "unsupported" before any
    /// entry; later failures are errors, so partial walks never merge.
    fn walk_hashed(
        &self,
        root: &str,
        want_hash: bool,
        tx: crossbeam_channel::Sender<HashHit>,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> VfsResult<bool> {
        super::backend_defaults::walk_hashed(root, want_hash, tx, cancel)
    }
}

pub type BackendHandle = Arc<dyn Backend>;
