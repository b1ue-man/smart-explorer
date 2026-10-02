use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

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
/// one instead.
pub(crate) fn syncfs(_path: &Path) -> std::io::Result<bool> {
    Ok(false)
}

/// Windows files carry ACLs inherited from their folder, not Unix modes.
pub(crate) fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

pub(crate) fn set_unix_mode(_file: &std::fs::File, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// Volume identity of `path` (volume serial number + location inside the
/// volume). `Ok(None)` = not determinable, treated as "unknown", never as
/// "another volume".
pub(crate) fn volume_identity(_path: &Path) -> std::io::Result<Option<super::VolumeIdentity>> {
    Ok(None)
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
