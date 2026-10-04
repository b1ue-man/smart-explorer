//! All variants are recoverable before the first replacement or exact-ID trash.
use super::duplicate_observation::{read_content, verify_content};
use super::duplicate_types::FileVariant;
use crate::vfs::Backend;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

pub(super) fn save(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    versions: &Path,
    variant: &FileVariant,
    cancel: &AtomicBool,
    throttle: &super::types::Throttle,
) -> io::Result<()> {
    super::duplicate_observation::check_cancel(cancel)?;
    let source = capture_variant(backend, path, rel, variant)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    for offset in 0..1000 {
        let target = versions
            .join(stamp.saturating_add(offset).to_string())
            .join(rel);
        if let Some(parent) = target.parent() {
            crate::support_dirs::ensure_private_dir(parent)?;
        }
        let mut file = match crate::support_dirs::create_private_file(&target) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let saved = (|| {
            verify_content(
                read_content(
                    backend,
                    path,
                    variant.id.as_deref(),
                    &mut file,
                    cancel,
                    Some(throttle),
                )?,
                variant,
            )?;
            super::apply_guard::revalidate(backend, path, &source, "backup variant")?;
            file.flush()?;
            file.sync_all()?;
            super::apply_stage::require_durable(super::apply_stage::native_namespace(&target)?)
        })();
        drop(file);
        if let Err(error) = saved {
            let _ = std::fs::remove_file(target);
            return Err(error);
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Kein freier Platz für eine Wiederherstellungsversion",
    ))
}

pub(super) fn save_scoped(
    side: &super::versions::VersionSide<'_>,
    path: &str,
    rel: &str,
    versions: &super::versions::RunVersions,
    variant: &FileVariant,
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::duplicate_observation::check_cancel(cancel)?;
    let captured = capture_variant(side.backend, path, rel, variant)?;
    let mut context = versions.context().clone();
    // Captured IDs cannot be addressed by rename(path) on a duplicate
    // provider. Keep each exact variant in the private fallback store.
    context.location = super::VersionsLocation::AppData;
    let copy_versions =
        super::versions::RunVersions::with_app_data(context, versions.app_data_dir().to_path_buf());
    copy_versions.bind_lock(&versions.lock_id())?;
    super::version_save::save(
        &copy_versions,
        side,
        path,
        rel,
        &captured,
        super::apply_guard::ExpectedFile::Present(variant.signature),
        super::versions::VersionReason::Resolved,
        cancel,
    )?;
    Ok(())
}

fn capture_variant(
    backend: &dyn Backend,
    path: &str,
    rel: &str,
    variant: &FileVariant,
) -> io::Result<super::apply_guard::CapturedFile> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let metadata = super::duplicate_observation::metadata_named(backend, path, name)?
        .into_iter()
        .find(|meta| meta.id == variant.id)
        .ok_or_else(super::duplicate_observation::changed)?;
    if metadata.size != variant.signature.size
        || metadata.mtime_ms != variant.signature.mtime_ms
        || metadata.content_md5.as_ref().is_some_and(|hash| {
            hash.len() == 32
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !hash.eq_ignore_ascii_case(&variant.content_md5)
        })
    {
        return Err(super::duplicate_observation::changed());
    }
    Ok(super::apply_guard::CapturedFile {
        metadata: Some(metadata),
    })
}
