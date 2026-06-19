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
pub(in crate::app) fn read_text(be: &dyn crate::vfs::Backend, path: &str) -> Result<String, String> {
    use std::io::Read;
    let mut r = be.open_read(path).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    // No line-diffing binary: reject invalid UTF-8 OR any NUL byte (a strong
    // binary signal even when the bytes happen to be valid UTF-8).
    if buf.contains(&0) {
        return Err("Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string());
    }
    String::from_utf8(buf).map_err(|_| "Keine Textdatei (binär) — bitte „A/B behalten“ nutzen.".to_string())
}

pub(in crate::app) fn write_bytes(be: &dyn crate::vfs::Backend, path: &str, data: &[u8]) -> Result<(), String> {
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
        if let Some(pid) = line.strip_prefix("pid=").and_then(|s| s.trim().parse().ok()) {
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
    read_session_pid(dir)
        .map(process_running)
        .unwrap_or(false)
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

pub(in crate::app) fn upload_file(be: &dyn crate::vfs::Backend, src: &std::path::Path, dest: &str) -> Result<(), String> {
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
pub(in crate::app) fn copy_remote_tree(be: &dyn crate::vfs::Backend, src: &str, dst: &str) -> std::io::Result<()> {
    let m = be.stat(src)?;
    if m.is_dir {
        be.mkdir_all(dst)?;
        for e in be.list_dir(src)? {
            let cs = format!("{}/{}", src.trim_end_matches('/'), e.name);
            let cd = format!("{}/{}", dst.trim_end_matches('/'), e.name);
            copy_remote_tree(be, &cs, &cd)?;
        }
        Ok(())
    } else {
        if let Some((parent, _)) = dst.rsplit_once('/') {
            let _ = be.mkdir_all(parent);
        }
        be.copy_file(src, dst).map(|_| ())
    }
}

pub(in crate::app) fn upload_dir(
    be: &dyn crate::vfs::Backend,
    dir: &std::path::Path,
    dest: &str,
    copied: &mut u64,
    errors: &mut Vec<String>,
) {
    if let Err(e) = be.mkdir_all(dest) {
        errors.push(format!("{}: {}", dest, e));
        return;
    }
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            errors.push(format!("{}: {}", dir.display(), e));
            return;
        }
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child = rjoin(dest, &name);
        let path = entry.path();
        if path.is_dir() {
            upload_dir(be, &path, &child, copied, errors);
        } else {
            match upload_file(be, &path, &child) {
                Ok(_) => *copied += 1,
                Err(e) => errors.push(format!("{}: {}", name, e)),
            }
        }
    }
}

/// Upload a set of local paths (files/folders) into `dest_root` on the backend.
/// Returns (files uploaded, error messages). Conflicts overwrite by name.
pub(in crate::app) fn upload_paths(
    be: &dyn crate::vfs::Backend,
    paths: &[String],
    dest_root: &str,
) -> (u64, Vec<String>) {
    let mut copied = 0u64;
    let mut errors = Vec::new();
    for p in paths {
        let src = std::path::PathBuf::from(p);
        let base = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if base.is_empty() {
            continue;
        }
        let dest = rjoin(dest_root, &base);
        if src.is_dir() {
            // Bulk fast path: stream the whole folder in one agent session
            // (no per-file round-trip). Falls back to the recursive per-file
            // upload on plain SFTP / on any agent error.
            if be.supports_bulk_tree() {
                match be.put_tree(&src, &dest) {
                    Ok(n) => {
                        copied += n;
                        continue;
                    }
                    Err(_) => {} // fall through to the per-file walk
                }
            }
            upload_dir(be, &src, &dest, &mut copied, &mut errors);
        } else {
            match upload_file(be, &src, &dest) {
                Ok(_) => copied += 1,
                Err(e) => errors.push(format!("{}: {}", base, e)),
            }
        }
    }
    (copied, errors)
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
