use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::types::{WireMeta, WireNode};

/// Linux pseudo-filesystems whose files report bogus huge sizes.
pub fn is_pseudo_dir(path: &str) -> bool {
    let p = path.trim_end_matches('/');
    matches!(p, "/proc" | "/sys" | "/dev" | "/run")
        || p.starts_with("/proc/")
        || p.starts_with("/sys/")
        || p.starts_with("/dev/")
        || p.starts_with("/run/")
}

pub(crate) fn systemtime_ms(t: std::time::SystemTime) -> i64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// List one directory's entries.
pub fn list_local(path: &str) -> std::io::Result<Vec<WireMeta>> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(path)? {
        let ent = ent?;
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let md = ent.metadata().ok();
        out.push(WireMeta {
            name: ent.file_name().to_string_lossy().into_owned(),
            is_dir: ft.is_dir(),
            is_symlink: ft.is_symlink(),
            size: md.as_ref().map(|m| m.len()).unwrap_or(0),
            mtime_ms: md
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(systemtime_ms)
                .unwrap_or(0),
        });
    }
    Ok(out)
}

/// Metadata for a single path.
pub fn stat_local(path: &str) -> std::io::Result<WireMeta> {
    let p = Path::new(path);
    let md = std::fs::symlink_metadata(p)?;
    let ft = md.file_type();
    Ok(WireMeta {
        name: p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        is_dir: md.is_dir(),
        is_symlink: ft.is_symlink(),
        size: md.len(),
        mtime_ms: md.modified().ok().map(systemtime_ms).unwrap_or(0),
    })
}

/// Recursive size walk, run locally on the server.
pub fn walk_local(root: &Path) -> WireNode {
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    walk_dir(root, name)
}

fn walk_dir(dir: &Path, name: String) -> WireNode {
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let nm = ent.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                let cp = ent.path();
                if is_pseudo_dir(&cp.to_string_lossy()) {
                    continue;
                }
                subdirs.push((cp, nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own += sz;
                files.push(WireNode {
                    name: nm,
                    size: sz,
                    is_dir: false,
                    children: Vec::new(),
                });
            }
        }
    }
    let mut dir_nodes: Vec<WireNode> = if subdirs.len() > 1 {
        subdirs
            .into_par_iter()
            .map(|(p, n)| walk_dir(&p, n))
            .collect()
    } else {
        subdirs.into_iter().map(|(p, n)| walk_dir(&p, n)).collect()
    };
    let mut size = own;
    for d in &dir_nodes {
        size += d.size;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    WireNode {
        name,
        size,
        is_dir: true,
        children,
    }
}

/// Live counters for a `WalkTree`.
pub struct WalkCounter {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
}

pub(crate) fn walk_dir_counted(
    dir: &Path,
    name: String,
    cnt: &WalkCounter,
    cancel: &AtomicBool,
) -> WireNode {
    if cancel.load(Ordering::Relaxed) {
        return WireNode {
            name,
            size: 0,
            is_dir: true,
            children: Vec::new(),
        };
    }
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let ft = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            let nm = ent.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                let cp = ent.path();
                if is_pseudo_dir(&cp.to_string_lossy()) {
                    continue;
                }
                subdirs.push((cp, nm));
            } else if ft.is_file() {
                let sz = ent.metadata().map(|m| m.len()).unwrap_or(0);
                own += sz;
                cnt.files.fetch_add(1, Ordering::Relaxed);
                cnt.bytes.fetch_add(sz, Ordering::Relaxed);
                files.push(WireNode {
                    name: nm,
                    size: sz,
                    is_dir: false,
                    children: Vec::new(),
                });
            }
        }
    }
    let mut dir_nodes: Vec<WireNode> = if subdirs.len() > 1 {
        subdirs
            .into_par_iter()
            .map(|(p, n)| walk_dir_counted(&p, n, cnt, cancel))
            .collect()
    } else {
        subdirs
            .into_iter()
            .map(|(p, n)| walk_dir_counted(&p, n, cnt, cancel))
            .collect()
    };
    let mut size = own;
    for d in &dir_nodes {
        size += d.size;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    WireNode {
        name,
        size,
        is_dir: true,
        children,
    }
}
