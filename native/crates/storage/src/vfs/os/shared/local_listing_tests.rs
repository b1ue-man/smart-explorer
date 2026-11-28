//! Listings of the local backend: a vanished entry is left out, a failure of
//! the enumeration itself is not (U18); a name that is no path here fails the
//! strict listing and is named by the tolerant one (U9).
use super::*;
use crate::vfs::BackendExtensions;

#[test]
fn wechselmedium_task_only_named_vanished_entries_are_left_out() {
    let named = EntryError::wrap(
        OsString::from("gone.txt"),
        Path::new("/medium/gone.txt"),
        io::Error::from(io::ErrorKind::NotFound),
    );
    assert!(vanished_entry(&named), "the entry itself disappeared");
    let directory = io::Error::from(io::ErrorKind::NotFound);
    assert!(
        !vanished_entry(&directory),
        "the directory's own failure (a removed medium) fails the listing"
    );
    let unreadable = EntryError::wrap(
        OsString::from("locked.txt"),
        Path::new("/medium/locked.txt"),
        io::Error::from(io::ErrorKind::PermissionDenied),
    );
    assert!(!vanished_entry(&unreadable));
}

#[test]
fn wechselmedium_task_listing_of_a_missing_directory_fails() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("removed-medium");
    let path = missing.to_string_lossy().replace('\\', "/");
    let local = LocalBackend::new("/");
    assert!(local.list_dir(&path).is_err(), "no silent empty listing");
    assert!(local.list_tolerant(&path).is_err());
}

#[cfg(unix)]
#[test]
fn wechselmedium_task_non_unicode_name_is_named_by_the_tolerant_listing() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("plain.txt"), b"plain").expect("plain");
    let raw = OsStr::from_bytes(b"bad-\xff-name.txt");
    std::fs::write(dir.path().join(raw), b"raw").expect("raw name");
    let path = dir.path().to_string_lossy().into_owned();
    let local = LocalBackend::new("/");
    assert!(
        local.list_dir(&path).is_err(),
        "the strict listing cannot represent the name"
    );
    let listing = local.list_dir_tolerant(&path).expect("tolerant listing");
    let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["plain.txt"]);
    assert_eq!(listing.omitted.len(), 1);
    assert_eq!(listing.omitted[0].reason, OmissionReason::Unrepresentable);
    assert!(listing.omitted[0].rel.starts_with("bad-"));
}
