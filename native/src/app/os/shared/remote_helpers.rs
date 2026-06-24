use super::prelude::*;
use super::*;

/// Line-merge editor state: a side-by-side aligned diff of the two versions.
pub(in crate::app) struct MergeUi {
    pub(in crate::app) rel: String,
    pub(in crate::app) rows: Vec<crate::linemerge::Row>,
}

pub(in crate::app) fn ep_join(root: &str, rel: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), rel)
}

/// Insert " (Konflikt <timestamp>)" before the extension of a relative path.
pub(in crate::app) fn conflict_rel_name(rel: &str) -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let seg_start = rel.rfind('/').map(|i| i + 1).unwrap_or(0);
    match rel[seg_start..].rfind('.') {
        Some(d) => {
            let dot = seg_start + d;
            format!("{} (Konflikt {}){}", &rel[..dot], ts, &rel[dot..])
        }
        None => format!("{} (Konflikt {})", rel, ts),
    }
}

/// Read a remote file as UTF-8 text (errors on binary), for the line-merge view.
pub(in crate::app) fn read_text(
    be: &dyn crate::vfs::Backend,
    path: &str,
) -> Result<String, String> {
    use std::io::Read;
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    // No line-diffing binary: reject invalid UTF-8 OR any NUL byte (a strong
    // binary signal even when the bytes happen to be valid UTF-8).
    if buf.contains(&0) {
        return Err("Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string());
    }
    String::from_utf8(buf)
        .map_err(|_| "Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string())
}

pub(in crate::app) fn write_bytes(
    be: &dyn crate::vfs::Backend,
    path: &str,
    data: &[u8],
) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = path.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut w = be.open_write(path).map_err(|e| e.to_string())?;
    w.write_all(data).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub(in crate::app) fn sig_from(be: &dyn crate::vfs::Backend, path: &str) -> crate::bisync::Sig {
    let m = be.stat(path).ok();
    crate::bisync::Sig {
        size: m.as_ref().map(|m| m.size).unwrap_or(0),
        mtime_ms: m.as_ref().map(|m| m.mtime_ms).unwrap_or(0),
        hash: 0,
    }
}

/// Root for all of this app's open/edit temp copies.
pub(in crate::app) fn temp_root() -> PathBuf {
    std::env::temp_dir().join("smart_explorer_open")
}

/// A stable tag unique to THIS process run (`<pid>_<start-nanos>`), so we can
/// tell our current session's temp dirs from stale ones left by prior runs.
pub(in crate::app) fn session_tag() -> &'static str {
    static T: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("s{}_{}", std::process::id(), nanos)
    })
}

pub(in crate::app) fn session_temp_dir() -> PathBuf {
    temp_root().join(session_tag())
}

pub(in crate::app) fn session_marker_path(dir: &Path) -> PathBuf {
    dir.join(TEMP_SESSION_PID_FILE)
}

pub(in crate::app) fn init_temp_session() {
    sweep_stale_temp();
    let _ = write_session_marker();
}

pub(in crate::app) fn write_session_marker() -> std::io::Result<()> {
    let dir = session_temp_dir();
    std::fs::create_dir_all(&dir)?;
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    std::fs::write(
        session_marker_path(&dir),
        format!(
            "pid={}\ntag={}\nstarted_ms={}\n",
            std::process::id(),
            session_tag(),
            started
        ),
    )
}

pub(in crate::app) fn read_session_pid(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(session_marker_path(dir)).ok()?;
    for line in text.lines() {
        if let Some(pid) = line
            .strip_prefix("pid=")
            .and_then(|s| s.trim().parse().ok())
        {
            return Some(pid);
        }
        if let Ok(pid) = line.trim().parse() {
            return Some(pid);
        }
    }
    None
}

#[cfg(windows)]
pub(in crate::app) fn process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
pub(in crate::app) fn process_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    Path::new("/proc").join(pid.to_string()).exists()
}

pub(in crate::app) fn session_dir_is_live(dir: &Path) -> bool {
    read_session_pid(dir).map(process_running).unwrap_or(false)
}

pub(in crate::app) fn safe_temp_name(name: &str) -> String {
    let safe = name.replace(['/', '\\', ':'], "_");
    if safe.trim().is_empty() {
        "datei".to_string()
    } else {
        safe
    }
}

/// A **fresh, unique** local path to download a remote file to for opening or
/// editing. Each call gets its own `<root>/<session>/<n>/<name>` subdir, so:
/// (1) two files with the same name never collide, and (2) a previous edit's
/// copy is never reused — every open is a clean download. Cleanup is by session
/// sweep (`sweep_stale_temp` at startup + `cleanup_session_temp` on exit), NOT
/// per-save: deleting a temp mid-edit silently loses changes in editors that
/// don't hold the file open (VS Code, Notepad, …) — see docs/vfs_research.
pub(in crate::app) fn open_temp_path(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = session_temp_dir();
    let _ = std::fs::create_dir_all(&root);
    let _ = write_session_marker();
    let safe = safe_temp_name(name);
    for _ in 0..16 {
        let mut bytes = [0u8; 8];
        if getrandom::getrandom(&mut bytes).is_ok() {
            let dir = root.join(format!("e{:016x}", u64::from_le_bytes(bytes)));
            match std::fs::create_dir(&dir) {
                Ok(()) => return dir.join(&safe),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => break,
            }
        }
    }
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = root.join(format!("e{}_{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&dir);
    dir.join(safe)
}

/// Remove leftover temp copies from PREVIOUS sessions (crash-safe net: TempDir-
/// style Drop cleanup never runs on a crash/kill, so a startup sweep is the
/// reliable guarantee). Never touches the current session's dir. Best-effort:
/// a dir whose file is still held open by an editor survives to a later sweep.
pub(in crate::app) fn sweep_stale_temp() {
    let cur = session_tag();
    if let Ok(rd) = std::fs::read_dir(temp_root()) {
        for e in rd.flatten() {
            if e.file_name().to_str() != Some(cur) && !session_dir_is_live(&e.path()) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// Delete this session's temp copies on a clean exit. Files an editor still
/// holds open won't delete (Windows) — those are caught by the next startup
/// sweep. Safe because we only ever delete on exit, never between saves.
pub(in crate::app) fn cleanup_session_temp() {
    let _ = std::fs::remove_dir_all(session_temp_dir());
}

pub(in crate::app) fn cleanup_temp_copy(temp: &Path) {
    if let Some(parent) = temp.parent() {
        if parent.starts_with(session_temp_dir()) {
            let _ = std::fs::remove_dir_all(parent);
            return;
        }
    }
    let _ = std::fs::remove_file(temp);
}

pub(in crate::app) fn file_mtime_ms(p: &std::path::Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
pub(in crate::app) struct EditProcess {
    pub(in crate::app) handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl EditProcess {
    pub(in crate::app) fn new(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Self> {
        if handle.is_null() {
            None
        } else {
            Some(Self { handle })
        }
    }

    pub(in crate::app) fn is_finished(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }
}

#[cfg(windows)]
impl Drop for EditProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// A remote file opened for editing in **temp mode**: the temp copy is watched
/// and re-uploaded to `remote_path` on the backend whenever it's saved.
pub(in crate::app) struct RemoteEdit {
    pub(in crate::app) temp: PathBuf,
    pub(in crate::app) backend: crate::vfs::BackendHandle,
    pub(in crate::app) remote_path: String,
    pub(in crate::app) name: String,
    /// Last mtime uploaded/downloaded — a change above this is a save.
    pub(in crate::app) baseline_mtime: i64,
    /// mtime seen last poll (1-cycle debounce so we don't upload mid-write).
    pub(in crate::app) seen_mtime: i64,
    /// The remote file's mtime when we last synced it (download or upload).
    /// Before overwriting, we re-check the remote; if it advanced past this,
    /// it changed underneath us → conflict, don't clobber. 0 = unknown (skip).
    pub(in crate::app) remote_known_mtime: i64,
    pub(in crate::app) dirty: bool,
    pub(in crate::app) uploading: bool,
    #[cfg(windows)]
    pub(in crate::app) process: Option<EditProcess>,
}

/// Outcome of a save-back upload attempt (computed off-thread).
pub(in crate::app) enum SaveResult {
    /// Uploaded; carries the remote's new mtime to re-baseline against.
    Ok(i64),
    /// The remote changed since we downloaded it — NOT overwritten. Carries the
    /// remote's current mtime.
    Conflict(i64),
    /// Upload failed.
    Failed(String),
}

pub(in crate::app) fn rjoin(root: &str, name: &str) -> String {
    format!("{}/{}", root.trim_end_matches('/'), name)
}

/// Stream one local file to `dest` on the backend (creating parent dirs). The
/// `flush()` is essential — the Drive backend uploads on flush.
pub(in crate::app) fn remote_temp_path(dest: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{dest}.se-upload-{}-{nanos:x}.part", std::process::id())
}

pub(in crate::app) fn upload_file_direct(
    be: &dyn crate::vfs::Backend,
    src: &std::path::Path,
    dest: &str,
) -> Result<(), String> {
    use std::io::Write;
    if let Some((parent, _)) = dest.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut r, &mut w).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

pub(in crate::app) fn upload_file(
    be: &dyn crate::vfs::Backend,
    src: &std::path::Path,
    dest: &str,
) -> Result<(), String> {
    if !be.rename_overwrites() {
        return upload_file_direct(be, src, dest);
    }
    let tmp = remote_temp_path(dest);
    if let Err(e) = upload_file_direct(be, src, &tmp) {
        let _ = be.remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = be.rename(&tmp, dest) {
        let _ = be.remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

/// Recursively copy `src` → `dst` WITHIN one backend. Files go through
/// `copy_file` (server-local + instant on the agent; SFTP streams through the
/// client as a fallback), directories are recreated and walked. Used for
/// same-connection remote→remote copy/move so nothing round-trips via a temp.
struct UploadEntry {
    src: PathBuf,
    rel: String,
    size: u64,
}

struct RemoteFileEntry {
    src: String,
    rel: String,
    size: u64,
}

fn send_transfer_progress(
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &TransferProgress,
    last: &mut std::time::Instant,
    force: bool,
) {
    if force || last.elapsed().as_millis() >= 80 {
        let _ = tx.send(TransferMsg::Progress(progress.clone()));
        *last = std::time::Instant::now();
    }
}

fn collect_upload_entries(
    path: &Path,
    rel: String,
    files: &mut Vec<UploadEntry>,
    dirs: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            errors.push(format!("{}: {}", path.display(), e));
            return;
        }
    };
    if meta.is_dir() {
        dirs.push(rel.clone());
        let rd = match std::fs::read_dir(path) {
            Ok(rd) => rd,
            Err(e) => {
                errors.push(format!("{}: {}", path.display(), e));
                return;
            }
        };
        for entry in rd {
            match entry {
                Ok(entry) => {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let child_rel = if rel.is_empty() {
                        name
                    } else {
                        format!("{}/{}", rel, name)
                    };
                    collect_upload_entries(&entry.path(), child_rel, files, dirs, errors);
                }
                Err(e) => errors.push(format!("{}: {}", path.display(), e)),
            }
        }
    } else {
        files.push(UploadEntry {
            src: path.to_path_buf(),
            rel,
            size: meta.len(),
        });
    }
}

fn upload_file_direct_progress(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
) -> Result<(), String> {
    use std::io::{Read, Write};
    if let Some((parent, _)) = dest.rsplit_once('/') {
        let _ = be.mkdir_all(parent);
    }
    let mut r = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut w = be.open_write(dest).map_err(|e| e.to_string())?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        send_transfer_progress(tx, progress, last, false);
    }
    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

fn upload_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &Path,
    dest: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
) -> Result<(), String> {
    if !be.rename_overwrites() {
        return upload_file_direct_progress(be, src, dest, tx, progress, last);
    }
    let tmp = remote_temp_path(dest);
    if let Err(e) = upload_file_direct_progress(be, src, &tmp, tx, progress, last) {
        let _ = be.remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = be.rename(&tmp, dest) {
        let _ = be.remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

pub(in crate::app) fn upload_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_root: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    for p in paths {
        let src = PathBuf::from(p);
        let base = match src.file_name().map(|n| n.to_string_lossy().to_string()) {
            Some(base) if !base.is_empty() => base,
            _ => continue,
        };
        collect_upload_entries(&src, base, &mut files, &mut dirs, &mut errors);
    }

    dirs.sort();
    dirs.dedup();
    let bytes_total = files.iter().map(|f| f.size).sum();
    let mut progress = TransferProgress::new(
        TransferKind::Upload,
        "Lade hoch",
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        let dest = rjoin(dest_root, &dir);
        if let Err(e) = be.mkdir_all(&dest) {
            errors.push(format!("{}: {}", dest, e));
            progress.errors = errors.len() as u64;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        let dest = rjoin(dest_root, &file.rel);
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        match upload_file_progress(be, &file.src, &dest, tx, &mut progress, &mut last) {
            Ok(()) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.len() as u64;
                progress.files_done = progress.files_done.saturating_add(1);
                progress.bytes_done = progress.bytes_done.saturating_add(file.size);
            }
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.done = true;
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.len() as u64;
    let _ = tx.send(TransferMsg::Done { progress, errors });
}

fn collect_remote_entries(
    be: &dyn crate::vfs::Backend,
    src: &str,
    rel: String,
    files: &mut Vec<RemoteFileEntry>,
    dirs: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    let meta = match be.stat(src) {
        Ok(m) => m,
        Err(e) => {
            errors.push(format!("{}: {}", src, e));
            return;
        }
    };
    if meta.is_dir {
        dirs.push(rel.clone());
        let entries = match be.list_dir(src) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(format!("{}: {}", src, e));
                return;
            }
        };
        for entry in entries {
            let child_src = format!("{}/{}", src.trim_end_matches('/'), entry.name);
            let child_rel = if rel.is_empty() {
                entry.name
            } else {
                format!("{}/{}", rel, entry.name)
            };
            collect_remote_entries(be, &child_src, child_rel, files, dirs, errors);
        }
    } else {
        files.push(RemoteFileEntry {
            src: src.to_string(),
            rel,
            size: meta.size,
        });
    }
}

fn download_file_progress(
    be: &dyn crate::vfs::Backend,
    src: &str,
    dest: &Path,
    expected: u64,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    progress: &mut TransferProgress,
    last: &mut std::time::Instant,
) -> Result<String, String> {
    use std::io::{Read, Write};

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    ensure_local_space(dest, expected)?;
    let part = download_part_path(dest);
    cleanup_partial(&part);
    let mut r = be.open_read(src).map_err(|e| e.to_string())?;
    let mut f = match std::fs::File::create(&part) {
        Ok(f) => f,
        Err(e) => {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
    };
    let mut copied = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = match r.read(&mut buf) {
            Ok(n) => n,
            Err(e) => {
                cleanup_partial(&part);
                return Err(e.to_string());
            }
        };
        if n == 0 {
            break;
        }
        if let Err(e) = f.write_all(&buf[..n]) {
            cleanup_partial(&part);
            return Err(e.to_string());
        }
        copied = copied.saturating_add(n as u64);
        progress.bytes_done = progress.bytes_done.saturating_add(n as u64);
        send_transfer_progress(tx, progress, last, false);
    }
    if let Err(e) = f.flush().and_then(|_| f.sync_all()) {
        cleanup_partial(&part);
        return Err(e.to_string());
    }
    drop(f);
    if expected != 0 && copied != expected {
        cleanup_partial(&part);
        return Err(format!(
            "Download unvollstaendig: {} von {} Bytes",
            copied, expected
        ));
    }
    if let Err(e) = replace_file_atomic(&part, dest) {
        cleanup_partial(&part);
        return Err(e.to_string());
    }
    Ok(dest.to_string_lossy().to_string())
}

fn download_remote_dir_for_clipboard(
    be: &dyn crate::vfs::Backend,
    src: &str,
    local_dir: &Path,
) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(local_dir);
    std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
    if be.supports_bulk_tree() {
        match be.get_tree(src, local_dir) {
            Ok(_) => return Ok(()),
            Err(_) => {
                let _ = std::fs::remove_dir_all(local_dir);
                std::fs::create_dir_all(local_dir).map_err(|e| e.to_string())?;
            }
        }
    }

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    collect_remote_entries(be, src, String::new(), &mut files, &mut dirs, &mut errors);
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        std::fs::create_dir_all(local_dir.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .map_err(|e| e.to_string())?;
    }
    for file in files {
        let dest = local_dir.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        download_to(be, &file.src, &dest)?;
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(in crate::app) fn download_remote_clipboard_items(
    be: &dyn crate::vfs::Backend,
    items: &[(String, String, bool)],
) -> Vec<String> {
    let mut local = Vec::new();
    for (path, name, is_dir) in items {
        if *is_dir {
            let local_dir = open_temp_path(name);
            if download_remote_dir_for_clipboard(be, path, &local_dir).is_ok() {
                local.push(local_dir.to_string_lossy().to_string());
            } else {
                let _ = std::fs::remove_dir_all(&local_dir);
            }
        } else {
            let local_name = be.download_name(path, name);
            if let Ok(p) = download_to_temp(be, path, &local_name) {
                local.push(p);
            }
        }
    }
    local
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "se_clip_test_{}_{}_{}",
            tag,
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn fwd(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn remote_clipboard_downloads_folder_tree() {
        let remote = temp_dir("remote");
        std::fs::create_dir_all(remote.join("Gate/sub")).unwrap();
        std::fs::write(remote.join("Gate/a.txt"), b"alpha").unwrap();
        std::fs::write(remote.join("Gate/sub/b.txt"), b"beta").unwrap();
        let be = crate::vfs::LocalBackend::new(&fwd(&remote));
        let item = (format!("{}/Gate", fwd(&remote)), "Gate".to_string(), true);

        let local = download_remote_clipboard_items(&be, &[item]);

        assert_eq!(local.len(), 1);
        let local_dir = PathBuf::from(&local[0]);
        assert!(local_dir.is_dir());
        assert_eq!(std::fs::read(local_dir.join("a.txt")).unwrap(), b"alpha");
        assert_eq!(std::fs::read(local_dir.join("sub/b.txt")).unwrap(), b"beta");

        let _ = std::fs::remove_dir_all(&remote);
        let _ = std::fs::remove_dir_all(local_dir);
    }
}

pub(in crate::app) fn download_paths_progress(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_local: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    let dest_root = PathBuf::from(dest_local.replace('/', std::path::MAIN_SEPARATOR_STR));
    for src in paths {
        let name = src
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("datei");
        collect_remote_entries(
            be,
            src,
            name.to_string(),
            &mut files,
            &mut dirs,
            &mut errors,
        );
    }
    dirs.sort();
    dirs.dedup();
    let bytes_total = files.iter().map(|f| f.size).sum();
    let mut progress = TransferProgress::new(
        TransferKind::Download,
        "Lade herunter",
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        let local = dest_root.join(dir.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Err(e) = std::fs::create_dir_all(&local) {
            errors.push(format!("{}: {}", local.display(), e));
            progress.errors = errors.len() as u64;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        let dest = dest_root.join(file.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        match download_file_progress(
            be,
            &file.src,
            &dest,
            file.size,
            tx,
            &mut progress,
            &mut last,
        ) {
            Ok(_) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.len() as u64;
                progress.files_done = progress.files_done.saturating_add(1);
                progress.bytes_done = progress.bytes_done.saturating_add(file.size);
            }
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.done = true;
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.len() as u64;
    let _ = tx.send(TransferMsg::Done { progress, errors });
}

pub(in crate::app) fn copy_remote_paths_progress(
    src: &dyn crate::vfs::Backend,
    paths: &[String],
    tgt: &dyn crate::vfs::Backend,
    dest_root: &str,
    same_server: bool,
    tx: &crossbeam_channel::Sender<TransferMsg>,
) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut errors = Vec::new();
    for src_path in paths {
        let name = src_path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("datei");
        collect_remote_entries(
            src,
            src_path,
            name.to_string(),
            &mut files,
            &mut dirs,
            &mut errors,
        );
    }
    dirs.sort();
    dirs.dedup();
    let file_bytes = files.iter().map(|f| f.size).sum::<u64>();
    let bytes_total = if same_server {
        file_bytes
    } else {
        file_bytes.saturating_mul(2)
    };
    let mut progress = TransferProgress::new(
        TransferKind::RemoteCopy,
        if same_server {
            "Kopiere remote"
        } else {
            "Uebertrage remote"
        },
        files.len() as u64,
        bytes_total,
    );
    progress.errors = errors.len() as u64;
    let mut last = std::time::Instant::now();
    send_transfer_progress(tx, &progress, &mut last, true);

    for dir in dirs {
        let dest = rjoin(dest_root, &dir);
        if let Err(e) = tgt.mkdir_all(&dest) {
            errors.push(format!("{}: {}", dest, e));
            progress.errors = errors.len() as u64;
        }
    }

    let start = std::time::Instant::now();
    for file in files {
        let dest = rjoin(dest_root, &file.rel);
        progress.current = file.rel.clone();
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
        let result = if same_server {
            if let Some((parent, _)) = dest.rsplit_once('/') {
                let _ = tgt.mkdir_all(parent);
            }
            tgt.copy_file(&file.src, &dest)
                .map(|_| {
                    progress.bytes_done = progress.bytes_done.saturating_add(file.size);
                })
                .map_err(|e| e.to_string())
        } else {
            let name = file.rel.rsplit('/').next().unwrap_or("datei");
            let tmp = open_temp_path(name);
            let downloaded = download_file_progress(
                src,
                &file.src,
                &tmp,
                file.size,
                tx,
                &mut progress,
                &mut last,
            );
            let uploaded = downloaded
                .and_then(|_| upload_file_progress(tgt, &tmp, &dest, tx, &mut progress, &mut last));
            cleanup_temp_copy(&tmp);
            uploaded
        };
        match result {
            Ok(()) => {
                progress.files_done = progress.files_done.saturating_add(1);
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.rel, e));
                progress.errors = errors.len() as u64;
                progress.files_done = progress.files_done.saturating_add(1);
                let missing = if same_server {
                    file.size
                } else {
                    file.size.saturating_mul(2)
                };
                progress.bytes_done = progress.bytes_done.saturating_add(missing);
            }
        }
        progress.elapsed_ms = start.elapsed().as_millis() as u64;
        send_transfer_progress(tx, &progress, &mut last, true);
    }

    progress.done = true;
    progress.elapsed_ms = start.elapsed().as_millis() as u64;
    progress.errors = errors.len() as u64;
    let _ = tx.send(TransferMsg::Done { progress, errors });
}

/// A bare drive letter like `C:` is **drive-relative** on Windows (it means
/// "current dir on C:"), so `read_dir("C:")` lists the wrong folder. Normalize
/// it to the drive root `C:/`.
pub(in crate::app) fn ensure_dir_root(p: &str) -> String {
    let t = p.trim();
    let b = t.as_bytes();
    if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("{}/", t)
    } else {
        t.to_string()
    }
}

pub(crate) fn is_local_style(path: &str) -> bool {
    let p = path.trim_start();
    let b = p.as_bytes();
    let has_drive = b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic();
    has_drive || p.starts_with("//") || p.starts_with("\\\\")
}

/// A ZIP archive we can browse in-app / extract.
pub(in crate::app) fn is_zip_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}
