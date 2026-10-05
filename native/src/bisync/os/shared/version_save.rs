//! Target-side archival rename, with a private app-data copy when unavailable.
use super::apply_guard::{capture, revalidate, CapturedFile, ExpectedFile};
use super::paths::join;
use super::transfer_stream::check;
use super::types::{Sig, Throttle, VersionsLocation};
use super::version_manifest::{self as record, Manifest};
use super::versions::{RunVersions, VersionReason, VersionSide};
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) struct Preserved {
    pub(super) moved: bool,
    pub(super) path: String,
    pub(super) signature: Sig,
}
pub(super) fn save(
    versions: &RunVersions,
    side: &VersionSide<'_>,
    path: &str,
    rel: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    reason: VersionReason,
    cancel: &AtomicBool,
) -> io::Result<Preserved> {
    check(cancel)?;
    record::validate_pair(&versions.context().pair_id)?;
    super::apply_boundary::guard(side.backend, side.root, rel, true)?;
    super::sync_relative_path::SyncRelativePath::parse(rel)?;
    let meta = captured.regular("version source")?;
    let signature = Sig {
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        hash: meta
            .content_md5
            .as_deref()
            .map(super::snapshot_hash::md5_hex_to_u64)
            .unwrap_or(expected.hash()),
    };
    // A provider with duplicate names needs an ID; rename(path) would select
    // an arbitrary member. The fallback copies precisely the captured ID.
    if versions.context().location == VersionsLocation::Auto
        && !side.backend.has_duplicate_file_names()
        && side.backend.download_name(path, &meta.name) == meta.name
    {
        match archive(
            versions, side, path, rel, captured, expected, reason, signature, cancel,
        ) {
            Ok(preserved) => return Ok(preserved),
            Err(error) if fallback_allowed(&error) => {
                // Never fall back after an ambiguous or committed rename.
                revalidate(side.backend, path, captured, "version source")?;
            }
            Err(error) => return Err(error),
        }
    }
    copy_private(
        versions, side, path, rel, captured, expected, reason, signature, cancel,
    )
}

#[allow(clippy::too_many_arguments)]
fn archive(
    versions: &RunVersions,
    side: &VersionSide<'_>,
    path: &str,
    rel: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    reason: VersionReason,
    mut signature: Sig,
    cancel: &AtomicBool,
) -> io::Result<Preserved> {
    let replica = replica(side.backend, side.root)?;
    let relative = format!(
        ".se-versions/{}/{}/{}",
        versions.run_id(),
        side_token(versions, side, &replica),
        record::random()?
    );
    let dir = record::ensure_dirs(side.backend, side.root, &relative)?;
    let data = join(&dir, "data");
    let meta = captured.regular("version source")?;
    let mut digest = meta.content_md5.clone();
    // Native provider hashes and stable metadata/ID permit a server-side
    // archive without downloading the old payload. A planned content hash
    // still demands verification when no native hash is available.
    if side.backend.is_local() || (expected.hash() != 0 && digest.is_none()) {
        let mut reader = crate::vfs::open_read_regular(side.backend, path, meta.id.as_deref())?;
        let content = super::transfer_stream::stream(
            &mut *reader,
            &mut io::sink(),
            cancel,
            None,
            expected.hash(),
            |_| {},
        )?;
        drop(reader);
        if content.bytes != meta.size
            || digest
                .as_ref()
                .is_some_and(|digest| !digest.eq_ignore_ascii_case(&content.hex()))
        {
            return Err(super::apply_guard::drift("version source content changed"));
        }
        signature.hash = content.hash();
        digest = Some(content.hex());
    }
    revalidate(side.backend, path, captured, "version source")?;
    let mut manifest = Manifest::new(
        versions,
        side,
        rel,
        reason,
        signature,
        data.clone(),
        replica,
    );
    manifest.data_id = meta.id.clone();
    manifest.digest = digest.clone();
    record::write(side.backend, &join(&dir, "intent.json"), &manifest)?;
    revalidate(side.backend, path, captured, "version source")?;
    check(cancel)?;
    crate::vfs::finish_stage(
        side.backend,
        path,
        crate::vfs::StageFinish {
            durability: crate::vfs::StageDurability::Now,
            ..Default::default()
        },
    )?;
    if let Err(error) = side.backend.rename_no_replace(path, &data) {
        // An error after a committed server-side rename must not permit a
        // private-copy fallback or an unsafe retry.
        if side.backend.try_exists(&data)? {
            return Err(io::Error::other(format!(
                "archive outcome uncertain: {error}"
            )));
        }
        return Err(error);
    }
    // The intention is already durable, so a crash leaves a discoverable
    // version even if the ready record cannot be completed.
    let finalized = (|| {
        super::apply_stage::require_durable(super::apply_stage::namespace(side.backend, &data)?)?;
        let archived = capture(
            side.backend,
            &data,
            ExpectedFile::Unknown,
            "archived version",
        )?;
        let actual = archived.regular("archived version")?;
        if actual.size != signature.size
            || actual
                .content_md5
                .as_ref()
                .zip(digest.as_ref())
                .is_some_and(|(actual, expected)| !actual.eq_ignore_ascii_case(expected))
        {
            return Err(super::apply_guard::drift(
                "archived version content changed",
            ));
        }
        manifest.data_id = actual.id.clone();
        manifest.data_sig = Sig {
            size: actual.size,
            mtime_ms: actual.mtime_ms,
            hash: signature.hash,
        };
        record::write(side.backend, &join(&dir, "entry.json"), &manifest)
    })();
    if let Err(error) = finalized {
        // Roll back only into an absent original; never overwrite a creator.
        if !side.backend.try_exists(path)? {
            let _ = side
                .backend
                .rename_no_replace(&data, path)
                .and_then(|()| super::apply_stage::namespace(side.backend, path).map(|_| ()));
        }
        return Err(io::Error::other(format!(
            "version could not be finalized: {error}"
        )));
    }
    Ok(Preserved {
        moved: true,
        path: data,
        signature: manifest.data_sig,
    })
}

#[allow(clippy::too_many_arguments)]
fn copy_private(
    versions: &RunVersions,
    side: &VersionSide<'_>,
    path: &str,
    rel: &str,
    captured: &CapturedFile,
    expected: ExpectedFile,
    reason: VersionReason,
    signature: Sig,
    cancel: &AtomicBool,
) -> io::Result<Preserved> {
    let root = versions.app_data_dir();
    crate::support_dirs::ensure_private_dir(root)?;
    let root = root
        .to_str()
        .ok_or_else(|| record::invalid("version path is not Unicode"))?;
    let backend = crate::vfs::LocalBackend::new(root);
    let replica = replica(side.backend, side.root)?;
    let relative = format!(
        "{}/{}/{}",
        versions.run_id(),
        side_token(versions, side, &replica),
        record::random()?
    );
    let dir = record::ensure_dirs(&backend, root, &relative)?;
    let mut current = std::path::PathBuf::from(root);
    for name in relative.split('/') {
        current.push(name);
        crate::support_dirs::ensure_private_dir(&current)?;
    }
    let data = join(&dir, "data");
    let mut manifest = Manifest::new(
        versions,
        side,
        rel,
        reason,
        signature,
        data.clone(),
        replica,
    );
    record::write(&backend, &join(&dir, "intent.json"), &manifest)?;
    let missing = CapturedFile { metadata: None };
    let staged = super::apply_stage::stage(
        side.backend,
        path,
        captured,
        expected,
        &backend,
        &data,
        &missing,
        crate::vfs::StageDurability::Now,
        &Throttle::new(0),
        cancel,
        |_| {},
    )?;
    let outcome = staged.publish(&data, &missing, true, cancel)?;
    super::apply_stage::require_durable(outcome.durable)?;
    let private_file = crate::support_dirs::open_private_file(std::path::Path::new(&data))?;
    crate::support_dirs::secure_private_file(&private_file)?;
    // Harden read-only backups before reopening for FlushFileBuffers, which
    // requires write access. Close the read pin for Windows share compatibility.
    drop(private_file);
    let private_file = crate::creds::private_storage::open_file(std::path::Path::new(&data), true)?;
    private_file.sync_all()?;
    manifest.size = outcome.bytes;
    manifest.digest = Some(
        outcome
            .digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    );
    manifest.data_sig = outcome.destination;
    manifest.data_id = backend.stat(&data)?.id;
    // This is the captured original metadata, with the digest of its actual
    // copied bytes, including provider exports whose metadata size is zero.
    manifest.original.hash = outcome.source.hash;
    record::write(&backend, &join(&dir, "entry.json"), &manifest)?;
    revalidate(side.backend, path, captured, "version source")?;
    Ok(Preserved {
        moved: false,
        path: data,
        signature: outcome.source,
    })
}
pub(super) fn rollback(
    side: &VersionSide<'_>,
    original: &str,
    preserved: &Preserved,
) -> io::Result<()> {
    if !preserved.moved {
        return Ok(());
    }
    let archived = capture(
        side.backend,
        &preserved.path,
        ExpectedFile::Present(preserved.signature),
        "rollback version",
    )?;
    archived.regular("rollback version")?;
    if side.backend.try_exists(original)? {
        return Ok(());
    }
    side.backend.rename_no_replace(&preserved.path, original)?;
    super::apply_stage::require_durable(super::apply_stage::namespace(side.backend, original)?)
}
pub(super) fn replica(backend: &dyn Backend, root: &str) -> io::Result<String> {
    let marker = crate::vfs::sync_child_path(backend, root, super::paths::REPLICA_MARKER_NAME)?;
    match backend.stat(&marker) {
        Ok(meta) if !meta.is_dir && !meta.is_symlink && !meta.special && meta.size <= 4096 => {
            use std::io::Read;
            let mut bytes = Vec::new();
            crate::vfs::open_read_regular(backend, &marker, meta.id.as_deref())?
                .take(4097)
                .read_to_end(&mut bytes)?;
            let parsed: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if let Some(id) = parsed
                .get("replica_id")
                .and_then(|id| id.as_str())
                .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                return Ok(format!("marker:{}", id.to_ascii_lowercase()));
            }
            return Err(record::invalid("invalid version replica marker"));
        }
        Ok(_) => return Err(record::invalid("invalid version replica marker")),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) => {}
        Err(error) => return Err(error),
    }
    Ok(crate::vfs::volume_identity(backend, root)?
        .map(|id| format!("volume:{}", id.key()))
        .unwrap_or_else(|| format!("location:{}:{root}", backend.state_identity())))
}
fn side_token(versions: &RunVersions, side: &VersionSide<'_>, replica: &str) -> String {
    format!(
        "{}-{}",
        side.side.as_str(),
        record::token(&format!("{}:{replica}", versions.context().pair_id))
    )
}
fn fallback_allowed(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::ReadOnlyFilesystem
    ) || error.raw_os_error() == Some(18) // cross-device rename
}
