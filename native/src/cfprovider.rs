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
        let mut r = self.backend.open_read(&remote).map_err(cerr)?;

        // Skip to the requested start offset.
        let mut to_skip = range.start;
        let mut sink = [0u8; 8192];
        while to_skip > 0 {
            let want = to_skip.min(sink.len() as u64) as usize;
            let n = r.read(&mut sink[..want]).map_err(cerr)?;
            if n == 0 {
                break;
            }
            to_skip -= n as u64;
        }
        // Read the requested length and hand it to the OS.
        let len = range.end.saturating_sub(range.start);
        let mut buf = Vec::new();
        r.take(len).read_to_end(&mut buf).map_err(cerr)?;
        ticket.write_at(&buf, range.start).map_err(cerr)?;
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
        let now = FileTime::now();
        let base = remote_dir.trim_end_matches('/');
        let mut placeholders: Vec<PlaceholderFile> = Vec::with_capacity(metas.len());
        for m in metas {
            let child_remote = format!("{}/{}", base, m.name);
            let md = if m.is_dir {
                Metadata::directory()
            } else {
                Metadata::file().size(m.size)
            }
            .created(now)
            .written(now);
            let mut pf = PlaceholderFile::new(&m.name)
                .mark_in_sync()
                .metadata(md)
                .blob(child_remote.into_bytes());
            if !m.is_dir {
                pf = pf.has_no_children();
            }
            placeholders.push(pf);
        }
        ticket.pass_with_placeholder(&mut placeholders).map_err(cerr)?;
        Ok(())
    }
}

/// Keep connections alive for the process lifetime (drop = disconnect).
fn registry() -> &'static Mutex<HashMap<String, Connection<RemoteFilter>>> {
    static R: OnceLock<Mutex<HashMap<String, Connection<RemoteFilter>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn provider_id(label: &str) -> String {
    let safe: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("SmartExplorer_{}", safe)
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
    if !sync_root_id.is_registered().map_err(|e| e.to_string())? {
        sync_root_id
            .register(
                SyncRootInfo::default()
                    .with_display_name(label)
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
    let conn = Session::new()
        .connect(&local_root, filter)
        .map_err(|e| e.to_string())?;
    registry().lock().unwrap().insert(key, conn);
    Ok(local_root)
}
