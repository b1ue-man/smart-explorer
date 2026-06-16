//! Remote file opening — "CfAPI / placeholder" mode (#23).
//!
//! Goal (see docs/REMOTE_EDIT.md): open a remote file at a **stable real local
//! path** that maps 1:1 to the remote, edit in any app, save back automatically.
//!
//! This module provides the **persistent per-connection sync folder** that mode
//! is built on: `%USERPROFILE%\Smart Explorer\<connection>\<remote path>`. The
//! file is hydrated there and watched for save-back (the app reuses its
//! edit-watch). Unlike the ephemeral temp mode, the path is stable and mirrors
//! the remote layout.
//!
//! The **native Windows Cloud Files API** layer (CfRegisterSyncRoot +
//! on-demand `CfExecute(TRANSFER_DATA)` hydration + OS save notifications, so
//! files are placeholders that download lazily and show the OneDrive-style
//! status) is the documented next step — it needs a real Windows test and is
//! tracked in docs/REMOTE_EDIT.md. The folder produced here is exactly the
//! sync-root location that layer will register.

use std::path::PathBuf;

/// Base directory for persistent remote mirrors.
pub fn sync_base() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("Smart Explorer")
}

fn san(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    let out = out.trim().trim_matches('.').to_string();
    if out.is_empty() { "_".to_string() } else { out }
}

/// The per-connection sync-root directory (`<base>/<connection>`).
pub fn conn_root_dir(conn_label: &str) -> PathBuf {
    sync_base().join(san(conn_label))
}

/// Stable local path for a remote file: `<base>/<connection>/<relative path>`,
/// where the relative path is the remote path under the connection root.
pub fn local_path(conn_label: &str, conn_root: &str, remote_path: &str) -> PathBuf {
    let root = conn_root.trim_end_matches('/');
    let rel = remote_path
        .strip_prefix(root)
        .unwrap_or(remote_path)
        .trim_start_matches('/');
    let mut p = sync_base().join(san(conn_label));
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        p = p.join(san(seg));
    }
    p
}

// ── native Windows Cloud Files API (best-effort) ─────────────────────────────
//
// Register the per-connection folder as a real CfAPI **sync root** so the OS
// treats it like OneDrive (status overlays, managed placeholders), and mark
// hydrated files in-sync. On-demand FETCH_DATA hydration (download lazily via a
// CfConnectSyncRoot callback) is the further step that needs a real Windows
// test — these calls are best-effort: if they fail the folder still works as a
// plain mirror. A fixed provider GUID identifies us.

#[cfg(windows)]
pub fn register_root(local_root: &std::path::Path) {
    use windows::core::{GUID, HSTRING, PCWSTR};
    use windows::Win32::Storage::CloudFilters::{
        CfRegisterSyncRoot, CF_REGISTER_FLAG_NONE, CF_SYNC_POLICIES, CF_SYNC_REGISTRATION,
    };
    let _ = std::fs::create_dir_all(local_root);
    let path = HSTRING::from(local_root.as_os_str());
    let name: Vec<u16> = "Smart Explorer\0".encode_utf16().collect();
    let ver: Vec<u16> = "1.0\0".encode_utf16().collect();
    let reg = CF_SYNC_REGISTRATION {
        StructSize: std::mem::size_of::<CF_SYNC_REGISTRATION>() as u32,
        ProviderName: PCWSTR(name.as_ptr()),
        ProviderVersion: PCWSTR(ver.as_ptr()),
        ProviderId: GUID::from_u128(0x5e_0a_5e_11_5e_0a_5e_0a_5e_0a_5e_0a_5e_0a_5e_01),
        ..Default::default()
    };
    let pol = CF_SYNC_POLICIES {
        StructSize: std::mem::size_of::<CF_SYNC_POLICIES>() as u32,
        ..Default::default()
    };
    unsafe {
        // Already-registered returns an error we deliberately ignore.
        let _ = CfRegisterSyncRoot(&path, &reg, &pol, CF_REGISTER_FLAG_NONE);
    }
}

/// Convert a freshly-hydrated file to an in-sync placeholder (best-effort).
#[cfg(windows)]
pub fn mark_in_sync(file: &std::path::Path) {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::CloudFilters::{
        CfConvertToPlaceholder, CfSetInSyncState, CF_CONVERT_FLAG_MARK_IN_SYNC,
        CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    let path = HSTRING::from(file.as_os_str());
    unsafe {
        let h: HANDLE = match CreateFileW(
            &path,
            GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        };
        let mut usn: i64 = 0;
        let _ = CfConvertToPlaceholder(
            h,
            None,
            0,
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            Some(&mut usn),
            None,
        );
        let _ = CfSetInSyncState(h, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, Some(&mut usn));
        let _ = CloseHandle(h);
    }
}

#[cfg(not(windows))]
pub fn register_root(_local_root: &std::path::Path) {}
#[cfg(not(windows))]
pub fn mark_in_sync(_file: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_path_mirrors_remote_layout() {
        let p = local_path("MyDrive", "/", "/Docs/Reports/q3.xlsx");
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.ends_with("Smart Explorer/MyDrive/Docs/Reports/q3.xlsx"), "{s}");
    }

    #[test]
    fn local_path_strips_connection_root() {
        let p = local_path("box", "/home/me", "/home/me/sub/file.txt");
        let s = p.to_string_lossy().replace('\\', "/");
        assert!(s.ends_with("Smart Explorer/box/sub/file.txt"), "{s}");
    }
}
