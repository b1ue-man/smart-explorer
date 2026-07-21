use rayon::prelude::*;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::types::{WireMeta, WireNode};

const MAX_WALK_NODES: u64 = 1_000_000;
const MAX_WALK_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 512;

#[derive(Default)]
struct WalkBudget {
    nodes: AtomicU64,
    text_bytes: AtomicU64,
}

impl WalkBudget {
    fn record(&self, name_bytes: u64, depth: usize) -> io::Result<()> {
        if depth > MAX_WALK_DEPTH {
            return Err(invalid("agent tree exceeds its depth limit"));
        }
        claim(&self.nodes, 1, MAX_WALK_NODES)
            .and_then(|_| claim(&self.text_bytes, name_bytes, MAX_WALK_TEXT_BYTES))
            .map_err(|_| invalid("agent tree exceeds its bounded collection budget"))
    }
}

fn claim(counter: &AtomicU64, amount: u64, maximum: u64) -> Result<(), ()> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(amount).filter(|next| *next <= maximum)
        })
        .map(|_| ())
        .map_err(|_| ())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn wire_name(name: std::ffi::OsString) -> io::Result<String> {
    name.into_string().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "filesystem name is not valid UTF-8",
        )
    })
}

fn wire_path(path: &Path) -> io::Result<&str> {
    path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "filesystem path is not valid UTF-8",
        )
    })
}

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
    let mut text_bytes = 0u64;
    for ent in std::fs::read_dir(path)? {
        let ent = ent?;
        let md = std::fs::symlink_metadata(ent.path())?;
        let name = wire_name(ent.file_name())?;
        text_bytes = text_bytes.saturating_add(name.len() as u64);
        if out.len() as u64 >= MAX_WALK_NODES || text_bytes > MAX_WALK_TEXT_BYTES {
            return Err(invalid(
                "directory listing exceeds its bounded collection budget",
            ));
        }
        out.push(WireMeta {
            name,
            is_dir: md.is_dir(),
            is_symlink: super::local_platform::metadata_is_link_like(&md),
            size: md.len(),
            mtime_ms: md.modified().ok().map(systemtime_ms).unwrap_or(0),
            content_md5: None,
        });
    }
    Ok(out)
}

/// Metadata for a single path.
pub fn stat_local(path: &str) -> std::io::Result<WireMeta> {
    let p = Path::new(path);
    let md = std::fs::symlink_metadata(p)?;
    Ok(WireMeta {
        name: p
            .file_name()
            .map(|s| wire_name(s.to_os_string()))
            .transpose()?
            .unwrap_or_else(|| path.to_string()),
        is_dir: md.is_dir(),
        is_symlink: super::local_platform::metadata_is_link_like(&md),
        size: md.len(),
        mtime_ms: md.modified().ok().map(systemtime_ms).unwrap_or(0),
        content_md5: None,
    })
}

/// Probe a local path without collapsing permission or I/O failures into
/// absence. Symlinks count as existing, including dangling symlinks.
pub(crate) fn try_exists_local(path: &str) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Recursive size walk, run locally on the server.
pub fn walk_local(root: &Path) -> io::Result<WireNode> {
    let name = match root.file_name() {
        Some(name) => wire_name(name.to_os_string())?,
        None => wire_path(root)?.to_string(),
    };
    let budget = WalkBudget::default();
    budget.record(name.len() as u64, 0)?;
    walk_dir(root, name, &budget, 0)
}

fn walk_dir(dir: &Path, name: String, budget: &WalkBudget, depth: usize) -> io::Result<WireNode> {
    require_plain_directory(dir)?;
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let metadata = std::fs::symlink_metadata(ent.path())?;
        if super::local_platform::metadata_is_link_like(&metadata) {
            continue;
        }
        let nm = wire_name(ent.file_name())?;
        budget.record(nm.len() as u64, depth.saturating_add(1))?;
        if metadata.is_dir() {
            let cp = ent.path();
            if is_pseudo_dir(wire_path(&cp)?) {
                continue;
            }
            subdirs.push((cp, nm));
        } else if metadata.is_file() {
            let sz = metadata.len();
            own = own
                .checked_add(sz)
                .ok_or_else(|| invalid("agent tree size overflow"))?;
            files.push(WireNode {
                name: nm,
                size: sz,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
    let mut dir_nodes: Vec<WireNode> = if subdirs.len() > 1 {
        subdirs
            .into_par_iter()
            .map(|(p, n)| walk_dir(&p, n, budget, depth.saturating_add(1)))
            .collect::<io::Result<Vec<_>>>()?
    } else {
        subdirs
            .into_iter()
            .map(|(p, n)| walk_dir(&p, n, budget, depth.saturating_add(1)))
            .collect::<io::Result<Vec<_>>>()?
    };
    let mut size = own;
    for d in &dir_nodes {
        size = size
            .checked_add(d.size)
            .ok_or_else(|| invalid("agent tree size overflow"))?;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    Ok(WireNode {
        name,
        size,
        is_dir: true,
        children,
    })
}

/// Live counters for a `WalkTree`.
pub struct WalkCounter {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    budget: WalkBudget,
}

impl WalkCounter {
    pub(crate) fn new() -> Self {
        Self {
            files: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            budget: WalkBudget::default(),
        }
    }
}

pub(crate) fn walk_dir_counted(
    dir: &Path,
    name: String,
    cnt: &WalkCounter,
    cancel: &AtomicBool,
) -> io::Result<WireNode> {
    cnt.budget.record(name.len() as u64, 0)?;
    walk_dir_counted_inner(dir, name, cnt, cancel, 0)
}

fn walk_dir_counted_inner(
    dir: &Path,
    name: String,
    cnt: &WalkCounter,
    cancel: &AtomicBool,
    depth: usize,
) -> io::Result<WireNode> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "agent tree walk canceled",
        ));
    }
    require_plain_directory(dir)?;
    let mut subdirs: Vec<(std::path::PathBuf, String)> = Vec::new();
    let mut files: Vec<WireNode> = Vec::new();
    let mut own = 0u64;
    for ent in std::fs::read_dir(dir)? {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "agent tree walk canceled",
            ));
        }
        let ent = ent?;
        let metadata = std::fs::symlink_metadata(ent.path())?;
        if super::local_platform::metadata_is_link_like(&metadata) {
            continue;
        }
        let nm = wire_name(ent.file_name())?;
        cnt.budget
            .record(nm.len() as u64, depth.saturating_add(1))?;
        if metadata.is_dir() {
            let cp = ent.path();
            if is_pseudo_dir(wire_path(&cp)?) {
                continue;
            }
            subdirs.push((cp, nm));
        } else if metadata.is_file() {
            let sz = metadata.len();
            own = own
                .checked_add(sz)
                .ok_or_else(|| invalid("agent tree size overflow"))?;
            cnt.files.fetch_add(1, Ordering::Relaxed);
            cnt.bytes
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bytes| {
                    bytes.checked_add(sz)
                })
                .map_err(|_| invalid("agent progress byte count overflow"))?;
            files.push(WireNode {
                name: nm,
                size: sz,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
    let mut dir_nodes = if subdirs.len() > 1 {
        subdirs
            .into_par_iter()
            .map(|(p, n)| walk_dir_counted_inner(&p, n, cnt, cancel, depth.saturating_add(1)))
            .collect::<io::Result<Vec<_>>>()?
    } else {
        subdirs
            .into_iter()
            .map(|(p, n)| walk_dir_counted_inner(&p, n, cnt, cancel, depth.saturating_add(1)))
            .collect::<io::Result<Vec<_>>>()?
    };
    let mut size = own;
    for d in &dir_nodes {
        size = size
            .checked_add(d.size)
            .ok_or_else(|| invalid("agent tree size overflow"))?;
    }
    let mut children = Vec::with_capacity(dir_nodes.len() + files.len());
    children.append(&mut dir_nodes);
    children.append(&mut files);
    Ok(WireNode {
        name,
        size,
        is_dir: true,
        children,
    })
}

fn require_plain_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if super::local_platform::metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "agent tree root changed into a link or non-directory: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn walk_budget_rejects_excessive_depth_without_building_a_tree() {
        let budget = WalkBudget::default();
        assert_eq!(
            budget.record(1, MAX_WALK_DEPTH + 1).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
