use super::{
    FileStore, ACCOUNT_DIGEST_BYTES, FORMAT_VERSION, MAGIC, MAX_RECORD_BYTES, MAX_SECRET_BYTES,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

fn fixture() -> (tempfile::TempDir, FileStore) {
    let root = tempfile::tempdir().unwrap();
    let store = FileStore::new(root.path().join("secrets-v1"));
    (root, store)
}

fn write_private(path: &Path, contents: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
}

#[test]
fn round_trip_survives_a_fresh_store_instance() {
    let (_root, store) = fixture();
    assert_eq!(store.get("account").unwrap(), None);
    store.set("account", "s3cr3t-ä").unwrap();
    assert_eq!(store.get("account").unwrap().as_deref(), Some("s3cr3t-ä"));

    let reopened = FileStore::new(store.directory.clone());
    assert_eq!(
        reopened.get("account").unwrap().as_deref(),
        Some("s3cr3t-ä")
    );
}

#[test]
fn created_directory_lock_and_record_have_exact_private_modes() {
    let (_root, store) = fixture();
    store.set("account", "secret").unwrap();
    let directory = fs::metadata(&store.directory).unwrap();
    let lock = fs::metadata(store.directory.join("store.lock")).unwrap();
    let record = fs::metadata(store.record_path("account")).unwrap();
    assert_eq!(directory.mode() & 0o7777, 0o700);
    assert_eq!(lock.mode() & 0o7777, 0o600);
    assert_eq!(record.mode() & 0o7777, 0o600);
    assert_eq!(lock.nlink(), 1);
    assert_eq!(record.nlink(), 1);
    assert!(fs::read_dir(&store.directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".stage-")));
}

#[test]
fn insecure_or_linked_store_directories_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let insecure = root.path().join("insecure");
    fs::create_dir(&insecure).unwrap();
    fs::set_permissions(&insecure, fs::Permissions::from_mode(0o755)).unwrap();
    let error = FileStore::new(insecure).get("account").unwrap_err();
    assert!(error.contains("0700"), "{error}");

    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    let linked = root.path().join("linked");
    symlink(target, &linked).unwrap();
    let error = FileStore::new(linked).set("account", "secret").unwrap_err();
    assert!(
        error.contains("open credential directory"),
        "unexpected error: {error}"
    );
}

#[test]
fn symlink_and_hard_link_records_fail_closed() {
    let (root, store) = fixture();
    assert_eq!(store.get("symlink").unwrap(), None);
    let outside = root.path().join("outside");
    write_private(&outside, b"outside");
    symlink(&outside, store.record_path("symlink")).unwrap();
    assert!(store
        .get("symlink")
        .unwrap_err()
        .contains("open credential record"));
    assert!(store
        .set("symlink", "new")
        .unwrap_err()
        .contains("open credential record"));
    assert!(store
        .delete("symlink")
        .unwrap_err()
        .contains("open credential record"));

    store.set("hardlink", "secret").unwrap();
    fs::hard_link(store.record_path("hardlink"), root.path().join("alias")).unwrap();
    assert!(store.get("hardlink").unwrap_err().contains("single-link"));
    assert!(store
        .set("hardlink", "new")
        .unwrap_err()
        .contains("single-link"));
    assert!(store
        .delete("hardlink")
        .unwrap_err()
        .contains("single-link"));
}

#[test]
fn unsafe_existing_record_mode_is_never_replaced_or_deleted() {
    let (_root, store) = fixture();
    store.set("account", "old").unwrap();
    let path = store.record_path("account");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.get("account").unwrap_err().contains("0600"));
    assert!(store.set("account", "new").unwrap_err().contains("0600"));
    assert!(store.delete("account").unwrap_err().contains("0600"));
    assert!(path.exists());
}

#[test]
fn corruption_truncation_and_oversize_are_explicit_errors() {
    let (_root, store) = fixture();
    store.set("corrupt", "secret").unwrap();
    let corrupt = store.record_path("corrupt");
    let mut bytes = fs::read(&corrupt).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    write_private(&corrupt, &bytes);
    assert!(store.get("corrupt").unwrap_err().contains("checksum"));

    store.set("truncated", "secret").unwrap();
    write_private(&store.record_path("truncated"), b"short");
    assert!(store.get("truncated").unwrap_err().contains("truncated"));

    assert_eq!(store.get("oversize").unwrap(), None);
    write_private(
        &store.record_path("oversize"),
        &vec![0u8; MAX_RECORD_BYTES + 1],
    );
    assert!(store.get("oversize").unwrap_err().contains("exceeds"));
    assert!(store
        .set("too-large", &"x".repeat(MAX_SECRET_BYTES + 1))
        .unwrap_err()
        .contains("exceeds"));
}

#[test]
fn unsupported_version_and_forged_length_are_rejected_before_use() {
    let (_root, store) = fixture();
    store.set("version", "secret").unwrap();
    let version = store.record_path("version");
    let mut bytes = fs::read(&version).unwrap();
    bytes[MAGIC.len()] = FORMAT_VERSION + 1;
    write_private(&version, &bytes);
    assert!(store.get("version").unwrap_err().contains("version"));

    store.set("length", "secret").unwrap();
    let length = store.record_path("length");
    let mut bytes = fs::read(&length).unwrap();
    let length_offset = MAGIC.len() + 1 + ACCOUNT_DIGEST_BYTES;
    bytes[length_offset..length_offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    write_private(&length, &bytes);
    assert!(store.get("length").unwrap_err().contains("exceeds"));
}

#[test]
fn copied_record_cannot_be_read_as_another_account() {
    let (_root, store) = fixture();
    store.set("first", "secret").unwrap();
    let bytes = fs::read(store.record_path("first")).unwrap();
    write_private(&store.record_path("second"), &bytes);
    let error = store.get("second").unwrap_err();
    assert!(error.contains("another account"), "{error}");
}

#[test]
fn replacement_and_idempotent_delete_preserve_store_health() {
    let (_root, store) = fixture();
    store.set("account", "old").unwrap();
    store.set("account", "new").unwrap();
    assert_eq!(store.get("account").unwrap().as_deref(), Some("new"));
    store.delete("account").unwrap();
    assert_eq!(store.get("account").unwrap(), None);
    store.delete("account").unwrap();
    store.set("account", "after-delete").unwrap();
    assert_eq!(
        store.get("account").unwrap().as_deref(),
        Some("after-delete")
    );
}

#[test]
fn empty_accounts_are_rejected_without_touching_the_store() {
    let (_root, store) = fixture();
    assert!(store.get("").unwrap_err().contains("must not be empty"));
    assert!(store
        .set("", "secret")
        .unwrap_err()
        .contains("must not be empty"));
    assert!(store.delete("").unwrap_err().contains("must not be empty"));
    assert!(!store.directory.exists());
}
