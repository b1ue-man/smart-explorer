//! Sequential optical path only when its metadata can be reproduced exactly.
//! Capability ambiguity deliberately keeps the serialized CopyFile2 path.
use super::{FileInformation, FileTime};
use std::{fs::File, io, os::windows::io::AsRawHandle, path::Path, sync::atomic::AtomicBool};
use windows_sys::Win32::Storage::FileSystem::*;

#[repr(C)]
struct BasicInfo {
    creation: i64,
    access: i64,
    write: i64,
    change: i64,
    attributes: u32,
}
fn ticks(time: &FileTime) -> i64 {
    ((u64::from(time.high) << 32) | u64::from(time.low)) as i64
}
pub(super) fn set_basic(file: &File, metadata: Option<&FileInformation>) -> io::Result<()> {
    let basic = match metadata {
        Some(metadata) => BasicInfo {
            creation: ticks(&metadata.creation_time),
            access: ticks(&metadata.last_access_time),
            write: ticks(&metadata.last_write_time),
            change: 0,
            attributes: metadata.attributes,
        },
        None => BasicInfo {
            creation: 0,
            access: 0,
            write: 0,
            change: 0,
            attributes: FILE_ATTRIBUTE_NORMAL,
        },
    };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileBasicInfo,
            (&basic as *const BasicInfo).cast(),
            std::mem::size_of::<BasicInfo>() as u32,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn copy(
    source: &Path,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Option<io::Result<Option<(u64, File)>>> {
    let flags = volume_flags(source).ok()?;
    // CopyFile2 can carry named streams, EAs, EFS, object IDs, security
    // resource attributes and filesystem-specific metadata. A filesystem
    // advertising these keeps that implementation, without a new loss.
    const COMPLEX: u32 = 0x0004_0000 | 0x0080_0000 | 0x0002_0000 | 0x0001_0000 | 0x0000_0008;
    if flags & COMPLEX != 0 {
        return None;
    }
    let reader =
        crate::local_access::open_regular(source, crate::local_access::FinalLink::Refuse).ok()?;
    let mut information = FileInformation::default();
    if unsafe { super::get_file_information_by_handle(reader.as_raw_handle(), &mut information) }
        == 0
    {
        return None;
    }
    // Only attributes SetFileAttributes documents as writable are handled;
    // compressed/sparse/encrypted/reparse/integrity sources retain CopyFile2.
    const SIMPLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x80 | 0x100 | 0x1000 | 0x2000;
    if information.attributes & !SIMPLE != 0 {
        return None;
    }
    Some(copy_simple(reader, &information, stage, cancel, progress))
}

fn copy_simple(
    mut reader: File,
    metadata: &FileInformation,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<Option<(u64, File)>> {
    // The former CopyFile2 stage inherited the destination parent's DACL.
    // Ordinary create_new retains precisely that destination-rights boundary.
    // Pinned like every Windows stage (`fast::create_stage`, A7): its
    // publication renames this handle.
    let stage_path = crate::local_access::normalize_scan_root(stage);
    let mut writer = super::fast::create_stage(stage)?;
    let result = (|| {
        let Some(bytes) =
            crate::source_media::sequential_copy(&mut reader, &mut writer, cancel, progress)?
        else {
            return Ok(None);
        };
        set_basic(&writer, Some(metadata))?;
        Ok(Some(bytes))
    })();
    match result {
        Ok(Some(bytes)) => Ok(Some((bytes, writer))),
        result => {
            // READONLY may have been set before a later failure. Clear only
            // our own exclusively created stage, never a foreign collision.
            let _ = set_basic(&writer, None);
            let own = super::file_identity(&writer)
                .and_then(|identity| super::path_matches_identity(&stage_path, identity))
                .unwrap_or(false);
            drop(writer);
            if own {
                let _ = std::fs::remove_file(stage_path);
            }
            result.map(|_| None)
        }
    }
}

fn volume_flags(path: &Path) -> io::Result<u32> {
    let path = super::wide(&crate::local_access::normalize_scan_root(path));
    let mut root = vec![0u16; 32768];
    if unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut flags = 0;
    if unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batches_task_optical_windows_sequential_metadata_and_cancel() {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source.bin");
        let stage = fixture.path().join("stage.bin");
        std::fs::write(
            &source,
            vec![37; crate::source_media::OPTICAL_BLOCK_BYTES + 19],
        )
        .unwrap();
        let source_wide = super::super::wide(&source);
        assert_ne!(
            unsafe {
                SetFileAttributesW(
                    source_wide.as_ptr(),
                    FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE,
                )
            },
            0
        );
        let reader =
            crate::local_access::open_regular(&source, crate::local_access::FinalLink::Refuse)
                .unwrap();
        let mut before = FileInformation::default();
        assert_ne!(
            unsafe {
                super::super::get_file_information_by_handle(reader.as_raw_handle(), &mut before)
            },
            0
        );
        let cancel = AtomicBool::new(false);
        let copied = copy_simple(reader, &before, &stage, &cancel, &mut |_| {})
            .unwrap()
            .unwrap();
        let mut after = FileInformation::default();
        assert_ne!(
            unsafe {
                super::super::get_file_information_by_handle(copied.1.as_raw_handle(), &mut after)
            },
            0
        );
        assert_eq!(ticks(&before.creation_time), ticks(&after.creation_time));
        assert_eq!(
            ticks(&before.last_access_time),
            ticks(&after.last_access_time)
        );
        assert_eq!(
            ticks(&before.last_write_time),
            ticks(&after.last_write_time)
        );
        assert_eq!(before.attributes, after.attributes);
        drop(copied);
        assert_eq!(
            std::fs::read(&stage).unwrap(),
            std::fs::read(&source).unwrap()
        );
        let canceled_stage = fixture.path().join("canceled.bin");
        let reader = File::open(&source).unwrap();
        let cancel = AtomicBool::new(true);
        assert!(
            copy_simple(reader, &before, &canceled_stage, &cancel, &mut |_| {})
                .unwrap()
                .is_none()
        );
        assert!(!canceled_stage.exists());
    }
}
