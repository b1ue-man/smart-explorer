//! Compare directory pins without truncating a provider's 128-bit identity.
use std::{fs::File, io, mem::size_of, os::windows::io::AsRawHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, FileNameInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO,
};

#[derive(PartialEq, Eq)]
pub(crate) enum Identity {
    Full(u64, [u8; 16]),
    Legacy(u32, u32, u32),
    /// A volume's root directory whose provider reports file ID 0: FAT
    /// reserves 0 for its root (fastfat `FatGenerateFileIdFromDirentOffset`,
    /// "Our root always has fileid 0"). A volume has exactly one root.
    Root(u64),
}

impl Identity {
    /// The text form kept in recovery records; the first two are unchanged.
    pub(crate) fn key(&self) -> String {
        match self {
            Identity::Full(volume, id) => format!(
                "windows-full:{volume}:{}",
                data_encoding::HEXLOWER.encode(id)
            ),
            Identity::Legacy(volume, high, low) => format!("windows:{volume}:{high}:{low}"),
            Identity::Root(volume) => format!("windows-root:{volume}"),
        }
    }
}

pub(crate) fn identity(file: &File) -> io::Result<Identity> {
    let mut full: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut full as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0
    {
        if full.FileId.Identifier == [0; 16] {
            return volume_root(file, full.VolumeSerialNumber);
        }
        return Ok(Identity::Full(
            full.VolumeSerialNumber,
            full.FileId.Identifier,
        ));
    }
    let error = io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124)) {
        return Err(error);
    }
    // Older/FAT providers retain their documented 64-bit identity. Do not
    // silently truncate an available ReFS/NTFS 128-bit ID.
    let mut legacy: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut legacy) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if legacy.nFileIndexHigh == 0 && legacy.nFileIndexLow == 0 {
        return volume_root(file, u64::from(legacy.dwVolumeSerialNumber));
    }
    Ok(Identity::Legacy(
        legacy.dwVolumeSerialNumber,
        legacy.nFileIndexHigh,
        legacy.nFileIndexLow,
    ))
}

/// File ID 0 identifies an object only as its volume's root, whose name
/// relative to the volume is exactly `\` (NtQueryInformationFile,
/// FileNameInformation: a full path begins with a single backslash). Any
/// other object with ID 0 has no identity.
fn volume_root(file: &File, volume: u64) -> io::Result<Identity> {
    if is_volume_root(file)? {
        return Ok(Identity::Root(volume));
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "provider has no directory identity",
    ))
}

fn is_volume_root(file: &File) -> io::Result<bool> {
    // FILE_NAME_INFO with room for one more unit than the root's `\`.
    #[repr(C)]
    struct Name {
        length: u32,
        units: [u16; 2],
    }
    let mut name = Name {
        length: 0,
        units: [0; 2],
    };
    // SAFETY: live handle and a correctly sized, aligned output record.
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileNameInfo,
            (&mut name as *mut Name).cast(),
            size_of::<Name>() as u32,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        // ERROR_MORE_DATA: the name is longer than the root's.
        return if error.raw_os_error() == Some(234) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    Ok(root_name(name.length, &name.units))
}

fn root_name(length: u32, units: &[u16]) -> bool {
    length == 2 && units.first() == Some(&u16::from(b'\\'))
}

pub(super) fn same_object(left: &File, right: &File) -> io::Result<bool> {
    Ok(identity(left)? == identity(right)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn removable_media_task_only_a_lone_backslash_names_the_volume_root() {
        let slash = u16::from(b'\\');
        assert!(root_name(2, &[slash, 0]));
        assert!(!root_name(0, &[0, 0]));
        assert!(!root_name(4, &[slash, u16::from(b'a')]));
        assert!(!root_name(2, &[u16::from(b'a'), 0]));
        assert_eq!(Identity::Root(7).key(), "windows-root:7");
        assert_eq!(Identity::Legacy(7, 0, 9).key(), "windows:7:0:9");
        assert_eq!(
            Identity::Full(7, [0xab; 16]).key(),
            format!("windows-full:7:{}", "ab".repeat(16))
        );
    }

    #[test]
    fn removable_media_task_system_volume_root_keeps_its_file_id() {
        // NTFS gives its root a nonzero ID; the root rule is FAT's case only.
        let root = std::env::var_os("SystemDrive").expect("SystemDrive");
        let mut path = std::path::PathBuf::from(root);
        path.push("\\");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
            .open(&path)
            .expect("open system root");
        assert!(is_volume_root(&file).expect("name query"));
        assert!(matches!(
            identity(&file).expect("identity"),
            Identity::Full(..)
        ));
        let windows = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS)
            .open(path.join("Windows"))
            .expect("open Windows folder");
        assert!(!is_volume_root(&windows).expect("name query"));
    }
}
