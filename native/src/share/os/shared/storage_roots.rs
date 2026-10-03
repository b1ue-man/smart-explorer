//! Resolve an admitted Share tree once; physical analysis roots do not overlap.
use std::{io, path::{Path, PathBuf}};
use crate::{analytics::Progress, share::{fs, fs_access::FsAccess}, vfs::Scheme};

pub(in crate::share) struct Root {
    pub visible: String,
    pub target: fs::ResolvedTarget,
    pub physical: Option<PathBuf>,
    pub retained: bool,
}
pub(in crate::share) enum Plan {
    Branch(String, Vec<Plan>), Leaf(usize),
    Failed { visible: String, kind: io::ErrorKind, detail: String },
}
impl Plan {
    pub(in crate::share) fn failures(&self, errors: &mut Vec<String>) {
        match self {
            Self::Branch(_, children) => for child in children { child.failures(errors); },
            Self::Failed { visible, detail, .. } => errors.push(format!("{visible}: {detail}")),
            Self::Leaf(_) => {},
        }
    }
}
pub(in crate::share) struct Roots { pub plan: Plan, pub roots: Vec<Root> }

pub(in crate::share) fn resolve(root: &str, access: &FsAccess, p: &Progress) -> io::Result<Roots> {
    let mut roots = Vec::new();
    let plan = collect(root, access, p, &mut roots)?;
    let mut order: Vec<usize> = (0..roots.len()).collect();
    order.sort_by_key(|i| roots[*i].physical.as_ref().map_or_else(
        || roots[*i].target.path.split('/').count(), |p| p.components().count()));
    let mut retained: Vec<usize> = Vec::new();
    for i in order {
        let duplicate = retained.iter().any(|j| overlaps(&roots[*j], &roots[i]));
        roots[i].retained = !duplicate;
        if !duplicate { retained.push(i); }
    }
    Ok(Roots { plan, roots })
}

fn collect(root: &str, access: &FsAccess, p: &Progress, roots: &mut Vec<Root>) -> io::Result<Plan> {
    p.check_cancel()?;
    let parts = fs::split_clean(root)?;
    let visible = format!("/{}", parts.join("/"));
    if access.is_dynamic() && (parts.is_empty() || parts == ["Verbindungen"]) {
        let mut children = Vec::new();
        for entry in access.list_dir(&visible)? {
            crate::vfs::validate_child_name(&entry.name)?;
            if entry.is_symlink || !entry.is_dir { continue; }
            children.push(collect(&format!("{}/{}", visible.trim_end_matches('/'), entry.name), access, p, roots)?);
        }
        return Ok(Plan::Branch(visible, children));
    }
    let target = match access.resolve(&visible) {
        Ok(target) => target,
        Err(error) => return Ok(Plan::Failed { visible, kind: error.kind(), detail: error.to_string() }),
    };
    // Only local targets use Path; distinct remote accounts retain their identity.
    let physical = if target.backend.scheme() == Scheme::Local {
        let path = match local_path(&target) {
            Ok(path) => path,
            Err(error) => return Ok(Plan::Failed { visible, kind: error.kind(), detail: error.to_string() }),
        };
        if is_private(&path) {
            return Ok(Plan::Failed { visible, kind: io::ErrorKind::PermissionDenied,
                detail: "Interner Host-Speicher ist keine freigegebene Benutzerdatei".into() });
        }
        Some(path)
    } else { None };
    let index = roots.len();
    roots.push(Root { visible, target, physical, retained: true });
    Ok(Plan::Leaf(index))
}

fn overlaps(parent: &Root, child: &Root) -> bool {
    match (&parent.physical, &child.physical) {
        (Some(parent), Some(child)) => child.starts_with(parent),
        (None, None) => parent.target.mount_key == child.target.mount_key
            && parent.target.backend.state_identity() == child.target.backend.state_identity()
            && (child.target.path == parent.target.path
                || child.target.path.strip_prefix(parent.target.path.trim_end_matches('/'))
                    .is_some_and(|rest| rest.starts_with('/'))),
        _ => false,
    }
}

pub(in crate::share) fn excluded() -> Vec<PathBuf> {
    let mut paths = vec![crate::support_dirs::app_data_dir()];
    if let Some(host) = crate::support_dirs::host() { paths.push(host.cache_dir.clone()); }
    paths.into_iter().map(|p| std::fs::canonicalize(&p).unwrap_or(p)).collect()
}

pub(in crate::share) fn is_private(path: &Path) -> bool {
    excluded().iter().any(|excluded| path.starts_with(excluded))
}

/// Canonicalize only the authorized export root. Descendants remain literal
/// components and are opened from its handle, never as a new free root.
fn local_parts(target: &fs::ResolvedTarget) -> io::Result<(PathBuf, PathBuf)> {
    let configured = crate::local_access::normalize_scan_root(Path::new(&target.backend.root_display()));
    let physical = std::fs::canonicalize(&configured).unwrap_or_else(|_| configured.clone());
    let requested = crate::local_access::normalize_scan_root(Path::new(&target.path));
    let relative = requested.strip_prefix(&physical).or_else(|_| requested.strip_prefix(&configured))
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "Lokales Ziel liegt außerhalb der autorisierten Exportwurzel"))?;
    if relative.components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "Lokales Ziel enthält keine literalen Kindkomponenten"));
    }
    let relative = relative.to_path_buf();
    Ok((physical, relative))
}
pub(in crate::share) fn local_path(target: &fs::ResolvedTarget) -> io::Result<PathBuf> {
    let (root, relative) = local_parts(target)?; Ok(root.join(relative))
}
pub(in crate::share) fn open_local(target: &fs::ResolvedTarget) -> io::Result<crate::local_access::DirectoryHandle> {
    let (root, relative) = local_parts(target)?;
    let mut directory = crate::local_access::DirectoryHandle::open_root(&root)?;
    for part in relative.components() {
        let std::path::Component::Normal(name) = part else { return Err(io::ErrorKind::PermissionDenied.into()); };
        directory = directory.open_child(name)?;
    }
    Ok(directory)
}
