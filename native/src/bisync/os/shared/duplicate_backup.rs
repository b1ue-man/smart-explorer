//! All variants are recoverable before the first replacement or exact-ID trash.
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use crate::vfs::Backend;
use super::duplicate_observation::{read_content, verify_content};
use super::duplicate_types::FileVariant;

pub(super) fn save(
    backend: &dyn Backend, path: &str, rel: &str, versions: &Path,
    variant: &FileVariant, cancel: &AtomicBool, throttle: &super::types::Throttle,
) -> io::Result<()> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs()).unwrap_or(0);
    for offset in 0..1000 {
        let target = versions.join(stamp.saturating_add(offset).to_string()).join(rel);
        if let Some(parent) = target.parent() { crate::support_dirs::ensure_private_dir(parent)?; }
        let mut file = match crate::support_dirs::create_private_file(&target) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let saved = (|| {
            verify_content(read_content(backend, path, variant.id.as_deref(), &mut file, cancel, Some(throttle))?, variant)?;
            file.flush()?;
            file.sync_all()?;
            let text = target.to_str().ok_or_else(|| io::Error::other("backup path is not Unicode"))?;
            let local = crate::vfs::LocalBackend::new(text);
            super::apply_stage::require_durable(super::apply_stage::namespace(&local, text)?)
        })();
        drop(file);
        if let Err(error) = saved {
            let _ = std::fs::remove_file(target);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "Kein freier Platz für eine Wiederherstellungsversion"))
}

pub(super) fn save_scoped(side: &super::versions::VersionSide<'_>, path: &str, rel: &str,
    versions: &super::versions::RunVersions, variant: &FileVariant, cancel: &AtomicBool,
) -> io::Result<()> {
    let parent = super::paths::parent_of(path).ok_or_else(|| io::Error::other("variant has no parent"))?;
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let meta = side.backend.list_dir_for_sync(&parent)?.into_iter()
        .find(|meta| meta.name == name && meta.id == variant.id)
        .ok_or_else(super::duplicate_observation::changed)?;
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err(super::apply_boundary::protected(if meta.is_symlink {
            super::OmissionKind::Link
        } else { super::OmissionKind::Special }));
    }
    let captured = super::apply_guard::CapturedFile { metadata: Some(meta) };
    let mut context = versions.context().clone();
    // Captured IDs cannot be addressed by rename(path) on a duplicate
    // provider. Keep each exact variant in the private fallback store.
    context.location = super::VersionsLocation::AppData;
    let copy_versions = super::versions::RunVersions::with_app_data(context, versions.app_data_dir().to_path_buf());
    copy_versions.bind_lock(&versions.lock_id())?;
    super::version_save::save(&copy_versions, side, path, rel, &captured,
        super::apply_guard::ExpectedFile::Present(variant.signature),
        super::versions::VersionReason::Resolved, cancel)?;
    Ok(())
}
