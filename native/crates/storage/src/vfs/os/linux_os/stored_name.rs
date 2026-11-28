//! The spelling a case-insensitive Linux filesystem stores for a name that
//! was looked up in another spelling (U2, 2026-11-27).
//!
//! FAT (`vfat`, `msdos`) and exFAT (the kernel driver and exfat-fuse) find
//! `Fotos` when asked for `fotos` (and `foto` for `foto.`: both drop trailing
//! dots of a looked-up name), while their listing reports `Fotos`. `stat`
//! reports the listed spelling, as Windows' `reported_name`
//! (`GetLongPathNameW`) does, so code that compares a `stat` with a listing
//! (the transfer preflight) compares names of one domain. Every other
//! filesystem, and every name without a letter or trailing dot, keeps the
//! spelling it was asked for unread. FAT and exFAT keep no name index: a
//! lookup there reads the directory entries one by one, so one more listing
//! keeps the cost of a lookup in the same order.
use std::ffi::{CString, OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::mountinfo;

/// `MSDOS_SUPER_MAGIC` (vfat, msdos), `EXFAT_SUPER_MAGIC`, `FUSE_SUPER_MAGIC`.
const MSDOS: u64 = 0x4d44;
const EXFAT: u64 = 0x2011_bab0;
const FUSE: u64 = 0x6573_5546;

pub(super) fn reported_name(path: &Path) -> Option<OsString> {
    let typed = path.file_name()?;
    if !may_be_stored_otherwise(typed) {
        return Some(typed.to_os_string());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !folds_names(parent) {
        return Some(typed.to_os_string());
    }
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Some(typed.to_os_string());
    };
    let listed = entries.filter_map(|entry| entry.ok().map(|entry| entry.file_name()));
    Some(stored_spelling(typed, listed).unwrap_or_else(|| typed.to_os_string()))
}

/// Only a letter (ASCII or beyond) or a trailing dot can make the stored
/// spelling differ from the looked-up one.
fn may_be_stored_otherwise(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.ends_with(b".")
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_alphabetic() || !byte.is_ascii())
}

/// Whether the filesystem holding `parent` matches names regardless of case.
/// FAT and exFAT, also through exfat-fuse (`fuseblk`), sit on a block device;
/// a filesystem on an anonymous device (major 0: tmpfs, Btrfs, overlay, NFS,
/// SMB, plain FUSE such as sshfs) is none of them and is not asked for its
/// type, which would cost a network filesystem a round trip per `stat`.
fn folds_names(parent: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(metadata) = std::fs::metadata(parent) else {
        return false;
    };
    if mountinfo::split_device(metadata.dev()).0 == 0 {
        return false;
    }
    match magic(parent) {
        Some(MSDOS | EXFAT) => true,
        Some(FUSE) => fuse_folds_names(parent),
        _ => false,
    }
}

fn magic(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut facts = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: a NUL-terminated path and a correctly sized output buffer.
    if unsafe { libc::statfs(path.as_ptr(), facts.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statfs filled the buffer on success; the magic is the low
    // 32 bits of a field whose width differs between targets.
    Some(unsafe { facts.assume_init() }.f_type as u64 & 0xffff_ffff)
}

/// A FUSE mount folds names when its driver is exfat-fuse (or a FAT driver):
/// told by the mount's subtype or, for `fuseblk`, by its block device's type.
fn fuse_folds_names(parent: &Path) -> bool {
    let Ok(resolved) = mountinfo::resolve_existing(parent) else {
        return false;
    };
    let Ok(mounts) = mountinfo::read() else {
        return false;
    };
    mountinfo::containing(&mounts, &resolved).is_some_and(|mount| {
        let block = super::volume_id::fuse_block_type(mount);
        fuse_type_folds(&mount.fs_type, block.as_deref())
    })
}

fn fuse_type_folds(fs_type: &str, block_type: Option<&str>) -> bool {
    let subtype = fs_type.split_once('.').map(|(_, subtype)| subtype);
    matches!(subtype, Some("exfat" | "exfat-fuse" | "vfat" | "fat"))
        || matches!(block_type, Some("exfat" | "vfat"))
}

/// The listed name `typed` resolves to on a folding filesystem: the exact
/// spelling if listed, else the single entry equal up to ASCII case (FAT's
/// rule; trailing dots of `typed` dropped), else the single entry equal up
/// to Unicode case (exFAT's upcase table). Several candidates name no
/// entry for sure: `None`, and the caller keeps the typed spelling.
fn stored_spelling(typed: &OsStr, listed: impl IntoIterator<Item = OsString>) -> Option<OsString> {
    let wanted = trim_trailing_dots(typed.as_bytes());
    let wanted_text = std::str::from_utf8(wanted).ok().map(str::to_uppercase);
    let mut ascii = Vec::new();
    let mut unicode = Vec::new();
    for name in listed {
        let bytes = name.as_bytes();
        if bytes == typed.as_bytes() {
            return Some(name);
        }
        if bytes.eq_ignore_ascii_case(wanted) {
            ascii.push(name);
        } else if let (Some(wanted), Ok(text)) = (&wanted_text, std::str::from_utf8(bytes)) {
            if text.to_uppercase() == *wanted {
                unicode.push(name);
            }
        }
    }
    let single =
        |mut candidates: Vec<OsString>| (candidates.len() == 1).then(|| candidates.remove(0));
    if ascii.is_empty() {
        single(unicode)
    } else {
        single(ascii)
    }
}

fn trim_trailing_dots(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 1 && bytes[end - 1] == b'.' {
        end -= 1;
    }
    &bytes[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(names: &[&str]) -> Vec<OsString> {
        names.iter().map(OsString::from).collect()
    }

    /// U2: which listed spelling `stat` reports for a typed name on FAT and
    /// exFAT, and which FUSE mounts fold names at all.
    #[test]
    fn removable_task_stat_reports_the_stored_spelling() {
        let pick = |typed: &str, names: &[&str]| {
            stored_spelling(OsStr::new(typed), listed(names))
                .map(|name| name.into_string().unwrap())
        };
        assert_eq!(pick("fotos", &["a", "Fotos"]).as_deref(), Some("Fotos"));
        assert_eq!(
            pick("Fotos", &["fotos", "Fotos"]).as_deref(),
            Some("Fotos"),
            "exact first"
        );
        assert_eq!(
            pick("BERICHT.TXT.", &["Bericht.txt"]).as_deref(),
            Some("Bericht.txt")
        );
        assert_eq!(pick("änderung", &["Änderung"]).as_deref(), Some("Änderung"));
        // FAT folds ASCII only, so "Änderung" and "änderung" may both
        // exist: the entry equal up to ASCII case wins.
        assert_eq!(
            pick("ÄNDERUNG", &["änderung", "Änderung"]).as_deref(),
            Some("Änderung")
        );
        assert_eq!(pick("fotos", &["FOTOS", "Fotos"]), None, "ambiguous");
        assert_eq!(pick("fehlt", &["anders"]), None);
        assert_eq!(pick("...", &["..."]).as_deref(), Some("..."));

        assert!(!may_be_stored_otherwise(OsStr::new("2024-11_27")));
        assert!(may_be_stored_otherwise(OsStr::new("2024.")));
        assert!(may_be_stored_otherwise(OsStr::new("fotos")));
        assert!(may_be_stored_otherwise(OsStr::new("日記")));

        assert!(fuse_type_folds("fuseblk", Some("exfat")));
        assert!(fuse_type_folds("fuseblk.exfat", None));
        assert!(fuse_type_folds("fuse.exfat", None));
        assert!(!fuse_type_folds("fuseblk", Some("ntfs")));
        assert!(!fuse_type_folds("fuse.sshfs", None));
        assert!(!fuse_type_folds("fuseblk", None));
    }
}
