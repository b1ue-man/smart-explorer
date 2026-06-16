//! Native on-demand Cloud Files API provider (#30), built on the `cloud-filter`
//! crate (the working CfAPI sync-engine wrapper used like OneDrive). It mounts a
//! remote connection as a CfAPI **sync root**: directories populate on demand
//! (`fetch_placeholders` → `backend.list_dir`) and files hydrate on open
//! (`fetch_data` → `backend.open_read`). Each placeholder's blob stores the
//! remote path so hydration knows what to download.
//!
//! API usage follows the crate's own behavior test (sync_filter.rs) verbatim.
//! Windows-only. Save-back is handled by the app's edit-watch on the hydrated
//! file (see app.rs); this module owns browse + hydrate.

#![cfg(windows)]

use crate::vfs::BackendHandle;
use cloud_filter::error::{CResult, CloudErrorKind};
use cloud_filter::filter::{info, ticket, Request, SyncFilter};
use cloud_filter::metadata::Metadata;
use cloud_filter::placeholder_file::PlaceholderFile;
use cloud_filter::root::{
    Connection, HydrationType, PopulationType, SecurityId, Session, SyncRootIdBuilder, SyncRootInfo,
};
use cloud_filter::utility::WriteAt;
use nt_time::FileTime;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn cerr<E: std::fmt::Display>(_e: E) -> CloudErrorKind {
    CloudErrorKind::Unsuccessful
}

/// Unix milliseconds → NT `FileTime` (100-ns ticks since 1601). 0/negative =
/// unknown → fall back to now. (UNIX_EPOCH = 116_444_736_000_000_000 ticks.)
fn ft_from_ms(ms: i64) -> FileTime {
    if ms <= 0 {
        return FileTime::now();
    }
    let ticks = 116_444_736_000_000_000u64.saturating_add((ms as u64).saturating_mul(10_000));
    FileTime::new(ticks)
}

struct RemoteFilter {
    backend: BackendHandle,
    remote_root: String,
    local_root: PathBuf,
}

impl RemoteFilter {
    /// Map a local sync-root path to the corresponding remote path.
    fn remote_of(&self, local: &Path) -> String {
        let rel = local
            .strip_prefix(&self.local_root)
            .unwrap_or(local)
            .to_string_lossy()
            .replace('\\', "/");
        let root = self.remote_root.trim_end_matches('/');
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            root.to_string()
        } else {
            format!("{}/{}", root, rel)
        }
    }
}

impl SyncFilter for RemoteFilter {
    fn fetch_data(
        &self,
        request: Request,
        ticket: ticket::FetchData,
        info: info::FetchData,
    ) -> CResult<()> {
        let blob = request.file_blob();
        let remote = if blob.is_empty() {
            self.remote_of(&request.path())
        } else {
            String::from_utf8_lossy(blob).to_string()
        };
        let range = info.required_file_range();
        // CfAPI requires every TRANSFER_DATA chunk's Offset to be 4 KiB-aligned,
        // and its Length 4 KiB-aligned UNLESS the chunk ends at EoF (a short
        // read = genuine EoF, which is exempt). So serve an aligned superset of
        // the required range: start rounded down, end rounded up to 4 KiB. A raw
        // (unaligned) write is rejected with 0x8007017C and stalls the open.
        const ALIGN: u64 = 4096;
        let start = range.start & !(ALIGN - 1);
        let end = range.end.saturating_add(ALIGN - 1) & !(ALIGN - 1);
        let want = end.saturating_sub(start);
        let mut r = self.backend.open_read(&remote).map_err(cerr)?;

        // Skip to the aligned start offset.
        let mut to_skip = start;
        let mut sink = [0u8; 8192];
        while to_skip > 0 {
            let chunk = to_skip.min(sink.len() as u64) as usize;
            let n = r.read(&mut sink[..chunk]).map_err(cerr)?;
            if n == 0 {
                break;
            }
            to_skip -= n as u64;
        }
        // Read up to the aligned length; a short read lands exactly on EoF, whose
        // unaligned final length the OS permits.
        let mut buf = Vec::new();
        r.take(want).read_to_end(&mut buf).map_err(cerr)?;
        if !buf.is_empty() {
            ticket.write_at(&buf, start).map_err(cerr)?;
        }
        Ok(())
    }

    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        let remote_dir = self.remote_of(&request.path());
        let metas = self.backend.list_dir(&remote_dir).map_err(cerr)?;
        let base = remote_dir.trim_end_matches('/');
        let mut placeholders: Vec<PlaceholderFile> = Vec::with_capacity(metas.len());
        for m in metas {
            let child_remote = format!("{}/{}", base, m.name);
            let md = if m.is_dir {
                Metadata::directory()
            } else {
                Metadata::file().size(m.size)
            }
            .created(ft_from_ms(m.btime_ms))
            .written(ft_from_ms(m.mtime_ms));
            // Display name: backends that transform on read (Google-Docs export)
            // give it the right extension; then sanitize with the SAME `san` the
            // open side uses (cfsync::local_path_named) so the placeholder we
            // create and the path the app launches always agree — and so the
            // name can't contain an interior NUL (which would panic across the
            // FFI callback boundary). The blob keeps the true remote path.
            let raw = if m.is_dir {
                m.name.clone()
            } else {
                self.backend.download_name(&child_remote, &m.name)
            };
            let display = crate::cfsync::san(&raw);
            let mut pf = PlaceholderFile::new(&display).mark_in_sync().metadata(md);
            // FileIdentity is capped at 4 KiB; a longer remote path would panic
            // the crate's assert inside the callback. Skip the blob in that case
            // (fetch_data falls back to mapping the local path).
            let blob = child_remote.into_bytes();
            if blob.len() <= 4096 {
                pf = pf.blob(blob);
            }
            if !m.is_dir {
                pf = pf.has_no_children();
            }
            placeholders.push(pf);
        }
        ticket.pass_with_placeholder(&mut placeholders).map_err(cerr)?;
        Ok(())
    }
}

/// Force on-demand population of every directory level from the sync root down
/// to `target`'s parent, so the leaf placeholder exists before we open it.
///
/// CfAPI only materializes placeholders in `fetch_placeholders`, which the OS
/// invokes on directory **enumeration**. Opening a leaf that was never browsed
/// finds nothing on disk → `ShellExecute` is a silent no-op. Reading each
/// ancestor directory (FindFirstFile under the hood) triggers population of that
/// level synchronously, so by the end the target placeholder exists.
pub fn populate_to(local_root: &Path, target: &Path) {
    fn drain(dir: &Path) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for _ in rd.flatten() {}
        }
    }
    let parent = match target.parent() {
        Some(p) => p,
        None => return,
    };
    drain(local_root);
    if let Ok(rel) = parent.strip_prefix(local_root) {
        let mut dir = local_root.to_path_buf();
        for seg in rel.components() {
            dir = dir.join(seg);
            drain(&dir);
        }
    }
}

/// Keep connections alive for the process lifetime (drop = disconnect).
fn registry() -> &'static Mutex<HashMap<String, Connection<RemoteFilter>>> {
    static R: OnceLock<Mutex<HashMap<String, Connection<RemoteFilter>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn provider_id(label: &str) -> String {
    // Must be injective (distinct labels → distinct ids, or two connections
    // collide on one sync root) AND bounded (the assembled SyncRootId is capped
    // at 174 chars, and SyncRootIdBuilder::new panics over 255). A readable
    // prefix plus an FNV-1a hash of the FULL label satisfies both.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in label.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let safe: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect();
    format!("SmartExplorer_{}_{:016x}", safe, h)
}

/// Mount `remote_root` of `backend` as a CfAPI sync root at the per-connection
/// local folder. Idempotent. Returns the local sync-root path.
pub fn ensure_mounted(
    label: &str,
    backend: BackendHandle,
    remote_root: &str,
) -> Result<PathBuf, String> {
    let local_root = crate::cfsync::conn_root_dir(label);
    let key = local_root.to_string_lossy().to_string();
    if registry().lock().unwrap().contains_key(&key) {
        return Ok(local_root);
    }
    std::fs::create_dir_all(&local_root).map_err(|e| e.to_string())?;

    let sid = SecurityId::current_user().map_err(|e| e.to_string())?;
    let pid = provider_id(label);
    let sync_root_id = SyncRootIdBuilder::new(&pid).user_security_id(sid).build();
    let mut did_register = false;
    if !sync_root_id.is_registered().map_err(|e| e.to_string())? {
        did_register = true;
        // Registration requires a non-empty icon resource ("<module>,<index>");
        // an empty one fails with E_INVALIDARG ("icon cannot be empty"). We ship
        // no embedded icon, so use a standard folder icon from shell32.dll.
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let icon = format!("{}\\System32\\shell32.dll,4", sysroot);
        sync_root_id
            .register(
                SyncRootInfo::default()
                    .with_display_name(label)
                    .with_icon(icon)
                    .with_hydration_type(HydrationType::Full)
                    .with_population_type(PopulationType::Full)
                    .with_version("1.0.0")
                    .with_path(&local_root)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
    }
    let filter = RemoteFilter {
        backend,
        remote_root: remote_root.trim_end_matches('/').to_string(),
        local_root: local_root.clone(),
    };
    let conn = match Session::new().connect(&local_root, filter) {
        Ok(c) => c,
        Err(e) => {
            // Don't leave a registered-but-unconnected sync root behind — the
            // cloud filter would then reject normal file ops in that folder.
            // Only undo OUR registration; a root that already existed (another
            // connection / a prior session) must not be torn down here.
            if did_register {
                let _ = sync_root_id.unregister();
            }
            return Err(e.to_string());
        }
    };
    registry().lock().unwrap().insert(key, conn);
    Ok(local_root)
}
