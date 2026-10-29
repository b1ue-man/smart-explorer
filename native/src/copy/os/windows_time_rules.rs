//! Which file times `CopyFile2` gives its copy, so that the handle copy of
//! `windows_fast.rs` gives a small file exactly what `CopyFile2` gives a large
//! one (A7 follow-up, 2026-10-29).
//!
//! Microsoft Learn does not say: the pages of `CopyFile`, `CopyFileEx`,
//! `CopyFile2` and `COPYFILE2_EXTENDED_PARAMETERS` name extended attributes,
//! OLE storage, alternate data streams, security resource attributes and
//! file attributes as preserved, and no page names a file time (checked
//! 2026-10-29). So it is learned from the system itself: once per process, a
//! one-byte file in the temp folder gets an old creation time and an old,
//! different last access time, is copied by the same `kernel_copy_file` that
//! copies the large files, and each of the two times of the copy is compared
//! with the source's. The last write time is always carried (a copy that
//! lost it would break every comparison by modification time). Whatever
//! cannot be learned (no temp folder, a refused copy) leaves the handle copy
//! as it was: all three times of the source are set.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_NORMAL, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

use super::super::{FileInformation, FileTime};

/// File times (100 ns since 1601) of 2000-01-01 and 2001-01-01: distinct
/// from each other and decades from any copy made now.
const CREATION: u64 = 125_911_584_000_000_000;
const ACCESS: u64 = 126_227_808_000_000_000;
/// Two days: coarser than the resolution of any file system (FAT keeps the
/// access time as a date), far finer than the distance to "now".
const TOLERANCE: u64 = 2 * 86_400 * 10_000_000;

/// Which times of the source the copy carries besides the last write time.
#[derive(Clone, Copy)]
pub(super) struct Rules {
    creation: bool,
    access: bool,
}

impl Rules {
    const ALL: Rules = Rules {
        creation: true,
        access: true,
    };

    /// `source` with the times the copy does not carry cleared: a zero time
    /// leaves the field of the new file as the system made it
    /// (`windows_optical::set_basic`).
    pub(super) fn carry(self, source: &FileInformation) -> FileInformation {
        let keep = |time: &FileTime, carried: bool| {
            if carried {
                FileTime {
                    low: time.low,
                    high: time.high,
                }
            } else {
                FileTime::default()
            }
        };
        FileInformation {
            attributes: source.attributes,
            creation_time: keep(&source.creation_time, self.creation),
            last_access_time: keep(&source.last_access_time, self.access),
            last_write_time: keep(&source.last_write_time, true),
            ..FileInformation::default()
        }
    }
}

/// The rules of this system, learned on the first call (concurrent callers
/// wait for it).
pub(super) fn learned() -> Rules {
    static LEARNED: OnceLock<Rules> = OnceLock::new();
    *LEARNED.get_or_init(|| learn().unwrap_or(Rules::ALL))
}

fn ticks(time: &FileTime) -> u64 {
    (u64::from(time.high) << 32) | u64::from(time.low)
}

fn mark(value: u64) -> FileTime {
    FileTime {
        low: value as u32,
        high: (value >> 32) as u32,
    }
}

fn learn() -> Option<Rules> {
    let folder = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let name = |kind: &str| folder.join(format!("se-times-{}-{stamp}.{kind}", std::process::id()));
    let (source, copy) = (name("src"), name("dst"));
    let rules = compare(&source, &copy);
    let _ = std::fs::remove_file(&copy);
    let _ = std::fs::remove_file(&source);
    rules
}

fn compare(source: &Path, copy: &Path) -> Option<Rules> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(source)
        .ok()?;
    file.write_all(b"x").ok()?;
    let marks = FileInformation {
        attributes: FILE_ATTRIBUTE_NORMAL,
        creation_time: mark(CREATION),
        last_access_time: mark(ACCESS),
        ..FileInformation::default()
    };
    super::super::windows_optical::set_basic(&file, Some(&marks)).ok()?;
    drop(file);
    let cancel = AtomicBool::new(false);
    super::super::kernel_copy_file(source, copy, &cancel, &mut |_| {}, Some(1)).ok()??;
    let made = made_times(copy)?;
    let near = |time: &FileTime, expected: u64| ticks(time).abs_diff(expected) < TOLERANCE;
    Some(Rules {
        creation: near(&made.creation_time, CREATION),
        access: near(&made.last_access_time, ACCESS),
    })
}

fn made_times(path: &Path) -> Option<FileInformation> {
    let file: File = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
        .ok()?;
    super::information(&file).ok()
}
