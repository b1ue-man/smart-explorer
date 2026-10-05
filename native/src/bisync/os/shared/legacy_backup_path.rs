//! Pre-RV1 archives store native paths. Never reinterpret a provider literal
//! as a native separator, drive prefix or alternate stream in this old format.
use crate::vfs::{self, LocalBackend};
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn destination(root: &Path, stamp: u64, rel: &str) -> io::Result<PathBuf> {
    super::sync_relative_path::SyncRelativePath::parse(rel)?;
    let root_text = root
        .to_str()
        .ok_or_else(|| io::Error::other("legacy versions root is not Unicode"))?;
    let backend = LocalBackend::new(root_text);
    let limits = vfs::target_limits(&backend, root_text);
    let mut path = root.join(stamp.to_string());
    for name in rel.split('/') {
        // This is a physical archive path, unlike an immutable manifest's rel.
        vfs::validate_child_name(name)?;
        if limits.name_issue(name).is_some() {
            return Err(super::apply_boundary::protected(
                super::OmissionKind::NameImpossibleOnTarget,
            ));
        }
        path.push(name);
    }
    Ok(path)
}
