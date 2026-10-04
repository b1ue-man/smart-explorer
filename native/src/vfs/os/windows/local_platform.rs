use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

#[path = "local_writes.rs"]
mod local_writes;
#[cfg(test)]
#[path = "review_task_stage_tests.rs"]
mod review_task_stage_tests;
#[path = "volume_info.rs"]
mod volume_info;

pub(crate) use local_writes::{check_new_name, create_new_private, open_stage, replace_file};

const MAX_LONG_PATH_UNITS: usize = 32_768;

fn file_attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

pub(crate) fn local_attrs(meta: &std::fs::Metadata) -> (bool, bool) {
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let a = file_attributes(meta);
    (
        a & FILE_ATTRIBUTE_HIDDEN != 0,
        a & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}

pub(crate) fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    file_attributes(meta) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// The OS path for a forward-slash VFS path. A bare drive (`C:`) means its
/// root. When a component is one Win32 name resolution would not address
/// literally (a reserved device name such as `NUL`, a trailing dot or space,
/// a rejected character), the result is the verbatim `\\?\` form so every
/// `std::fs` call reaches the stored entry instead of the device or a
/// stripped name; ordinary paths keep their plain spelling.
pub(crate) fn to_os(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{}/", path);
        rooted.as_str()
    } else {
        path
    };
    let native = path.replace('/', std::path::MAIN_SEPARATOR_STR);
    match super::verbatim::verbatim_if_hostile(&native) {
        Some(verbatim) => PathBuf::from(verbatim),
        None => PathBuf::from(native),
    }
}

/// Key of the volume serving `path` for shared concurrency control: the drive
/// (`C:`) or the UNC share (`//server/share`), case-folded like Win32 names.
pub(crate) fn volume_key(path: &str) -> String {
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    let rest = lower
        .strip_prefix("//?/")
        .or_else(|| lower.strip_prefix("//./"))
        .unwrap_or(&lower);
    if let Some(unc) = rest.strip_prefix("unc/") {
        return unc_volume_key(unc);
    }
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return format!("local:{}", &rest[..2]);
    }
    match rest.strip_prefix("//") {
        Some(unc) => unc_volume_key(unc),
        None => "local:".to_string(),
    }
}

fn unc_volume_key(rest: &str) -> String {
    let mut parts = rest.split('/').filter(|part| !part.is_empty());
    match (parts.next(), parts.next()) {
        (Some(server), Some(share)) => format!("unc://{server}/{share}"),
        (Some(server), None) => format!("unc://{server}"),
        _ => "unc:".to_string(),
    }
}

/// Return the name stored by the filesystem rather than the spelling used to
/// address it. Windows can address one entry through both its long name and an
/// 8.3 alias (for example `runneradmin` and `RUNNER~1`), while `read_dir`
/// reports the stored long name. Keeping `stat` and `list_dir` in the same name
/// domain lets backend-neutral preflight code compare their results safely.
pub(crate) fn reported_name(path: &Path) -> Option<OsString> {
    long_path(path)
        .and_then(|long| long.file_name().map(OsStr::to_os_string))
        .or_else(|| path.file_name().map(OsStr::to_os_string))
}

fn long_path(path: &Path) -> Option<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetLongPathNameW;

    let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if input.len() > MAX_LONG_PATH_UNITS {
        return None;
    }
    let mut output = vec![0u16; input.len().max(1)];
    loop {
        let written = unsafe {
            GetLongPathNameW(
                input.as_ptr(),
                output.as_mut_ptr(),
                u32::try_from(output.len()).ok()?,
            )
        };
        let written = usize::try_from(written).ok()?;
        if written == 0 || written > MAX_LONG_PATH_UNITS {
            return None;
        }
        if written < output.len() {
            output.truncate(written);
            return Some(PathBuf::from(OsString::from_wide(&output)));
        }
        output.resize(written, 0);
    }
}

/// Windows offers no unprivileged filesystem-wide flush (a volume handle for
/// `FlushFileBuffers` needs administrator rights): stages are flushed one by
/// one and published by write-through renames, so nothing is left to do.
pub(crate) fn flush_filesystem(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Successful local publications already use write-through renames after
/// their file flush. Preserve that contract without a privileged volume flush.
pub(crate) fn confirm_namespace(path: &Path) -> std::io::Result<bool> {
    if !std::fs::metadata(path)?.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "namespace parent must be a directory",
        ));
    }
    if filesystem_profile(path)?.flush == super::fs_profile::FlushModel::PerFileOnly {
        return Ok(false);
    }
    flush_filesystem(path)?;
    Ok(true)
}

/// Windows files carry ACLs inherited from their folder, not Unix modes.
pub(crate) fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

pub(crate) fn set_unix_mode(_file: &std::fs::File, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Volume identity of `path`: 64-bit volume serial number plus the location
/// inside the volume (independent of drive letter, mount folder and
/// junctions on the way). `Ok(None)` = the volume has no serial number,
/// treated as "unknown", never as "another volume".
pub(crate) fn volume_identity(path: &Path) -> std::io::Result<Option<super::VolumeIdentity>> {
    let facts = volume_info::volume_facts(path)?;
    Ok((facts.serial != 0).then(|| super::VolumeIdentity {
        volume_id: format!("{:016x}", facts.serial),
        relative_path: facts.inside,
        fs_type: facts.fs_name,
    }))
}

/// What the volume holding `path` (or its nearest existing ancestor) can
/// store and how it flushes.
pub(crate) fn filesystem_profile(path: &Path) -> std::io::Result<super::fs_profile::FsProfile> {
    let mut current = path;
    loop {
        match volume_info::volume_facts(current) {
            Ok(facts) => {
                return Ok(super::fs_profile::windows_profile(
                    &facts.fs_name,
                    facts.max_component,
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match current.parent() {
                Some(parent) => current = parent,
                None => return Err(error),
            },
            Err(error) => return Err(error),
        }
    }
}

/// Win32 names remain constrained even when a volume cannot be queried.
pub(crate) fn fallback_limits() -> super::TargetLimits {
    super::fs_profile::windows_profile("", 0).limits
}

/// Stages are never batched on Windows, so no device number is needed.
pub(crate) fn device_of(_metadata: &std::fs::Metadata) -> Option<u64> {
    None
}

/// Volumes mounted in folders are junctions, which walks treat as links.
pub(crate) fn mount_boundary(_path: &Path) -> std::io::Result<Option<super::MountKind>> {
    Ok(None)
}

pub(crate) fn remove_file_like(path: &Path) -> std::io::Result<()> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let metadata = std::fs::symlink_metadata(path)?;
    if is_reparse_point(&metadata) && file_attributes(&metadata) & FILE_ATTRIBUTE_DIRECTORY != 0 {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    }
}

pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    // Unlike std::fs, direct Win32 calls need an explicit verbatim path for
    // long names even when the executable has no long-path manifest.
    let source = crate::local_access::normalize_scan_root(source);
    let destination = crate::local_access::normalize_scan_root(destination);
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Omitting MOVEFILE_REPLACE_EXISTING is the Win32 no-replace primitive.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::reported_name;

    #[test]
    fn reported_temp_ancestor_names_match_directory_listings() {
        let mut checked = 0usize;
        for path in std::env::temp_dir().ancestors() {
            let (Some(parent), Some(reported)) = (path.parent(), reported_name(path)) else {
                continue;
            };
            let found = std::fs::read_dir(parent)
                .unwrap_or_else(|error| panic!("cannot list {}: {error}", parent.display()))
                .filter_map(Result::ok)
                .any(|entry| entry.file_name() == reported);
            assert!(
                found,
                "{} was not reported as {:?} by its parent listing",
                path.display(),
                reported
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "the Windows temp path had no testable ancestor"
        );
    }
}
