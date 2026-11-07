use super::FileStore;
use crate::creds::key_protector::{protect_with_key_file, KeyProtector, Protected, APP_KEY_FILE};
use crate::creds::sealed_record::{
    encode_legacy, DataKey, KeyringDescriptor, MAX_RECORD_BYTES, MAX_SECRET_BYTES,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

#[path = "linux_file_store_port_tests.rs"]
mod port_tests;

/// Test protector: the `app-key-file` protector only, never `systemd-creds`
/// or a host key, so tests are independent of the machine running them.
struct KeyFileOnly;

impl KeyProtector for KeyFileOnly {
    fn new_key(&self) -> Result<DataKey, String> {
        DataKey::generate()
    }

    fn protect(&self, directory: &Path, key: &DataKey) -> Result<Protected, String> {
        protect_with_key_file(directory, key)
    }

    fn recover(&self, directory: &Path, descriptor: &KeyringDescriptor) -> Result<DataKey, String> {
        assert_eq!(descriptor.protector, APP_KEY_FILE);
        let kek: [u8; 32] = fs::read(crate::creds::key_protector::key_file_path(directory))
            .map_err(|error| error.to_string())?
            .try_into()
            .map_err(|_| "key file length".to_string())?;
        use base64::Engine as _;
        let wrapped = base64::engine::general_purpose::STANDARD
            .decode(descriptor.wrapped.as_bytes())
            .map_err(|error| error.to_string())?;
        let key = crate::creds::sealed_record::unwrap_key(&kek, &wrapped)?;
        crate::creds::key_protector::check_id(&key, descriptor)?;
        Ok(key)
    }
}

fn store_at(path: std::path::PathBuf) -> FileStore {
    FileStore::with_protector(path, Arc::new(KeyFileOnly))
}

fn fixture() -> (tempfile::TempDir, FileStore) {
    let root = tempfile::tempdir().unwrap();
    let store = store_at(root.path().join("secrets-v1"));
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

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn round_trip_survives_a_fresh_store_instance() {
    let (_root, store) = fixture();
    assert_eq!(store.get("account").unwrap(), None);
    store.set("account", "s3cr3t-ä").unwrap();
    assert_eq!(store.get("account").unwrap().as_deref(), Some("s3cr3t-ä"));

    let reopened = store_at(store.directory.clone());
    assert_eq!(
        reopened.get("account").unwrap().as_deref(),
        Some("s3cr3t-ä")
    );
}

#[test]
fn luecken_task_records_on_disk_contain_no_plaintext() {
    // Rows: a secret whose bytes must not reach the disk, and a second secret
    // for the file modes. Every row runs all checks: no plaintext in the record
    // or the descriptor, the key file at 0600, the directory at 0700, the lock
    // and the record at 0600 with one link each, and no stage files.
    for (case, secret) in [
        ("plaintext", "plain-text-password-123"),
        ("modes", "secret"),
    ] {
        let (root, store) = fixture();
        store.set("account", secret).expect(case);
        let record = fs::read(store.record_path("account")).expect(case);
        assert!(!contains(&record, secret.as_bytes()), "{case}");
        assert!(record.starts_with(b"SESEC02\0"), "{case}");
        let descriptor = fs::read(store.directory.join("keyring-v2")).expect(case);
        assert!(!contains(&descriptor, secret.as_bytes()), "{case}");
        let key_file = root.path().join("secrets-v1.kek");
        assert_eq!(
            fs::metadata(&key_file).expect(case).mode() & 0o7777,
            0o600,
            "{case}"
        );
        let directory = fs::metadata(&store.directory).expect(case);
        let lock = fs::metadata(store.directory.join("store.lock")).expect(case);
        let record_meta = fs::metadata(store.record_path("account")).expect(case);
        assert_eq!(directory.mode() & 0o7777, 0o700, "{case}");
        assert_eq!(lock.mode() & 0o7777, 0o600, "{case}");
        assert_eq!(record_meta.mode() & 0o7777, 0o600, "{case}");
        assert_eq!(lock.nlink(), 1, "{case}");
        assert_eq!(record_meta.nlink(), 1, "{case}");
        let stage_free = fs::read_dir(&store.directory).expect(case).all(|entry| {
            !entry
                .expect(case)
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-")
        });
        assert!(stage_free, "{case}");
    }
}

#[test]
fn luecken_task_plaintext_written_by_an_old_version_later_is_ported_on_read() {
    let (_root, store) = fixture();
    store.set("first", "sealed").unwrap();
    write_private(
        &store.record_path("late"),
        &encode_legacy("late", "written-by-old-binary"),
    );
    assert_eq!(
        store.get("late").unwrap().as_deref(),
        Some("written-by-old-binary")
    );
    let record = fs::read(store.record_path("late")).unwrap();
    assert!(record.starts_with(b"SESEC02\0"));
    assert!(!contains(&record, b"written-by-old-binary"));
}

#[test]
fn luecken_task_missing_key_file_fails_without_losing_records() {
    let (root, store) = fixture();
    store.set("account", "secret").unwrap();
    let key_file = root.path().join("secrets-v1.kek");
    let saved = fs::read(&key_file).unwrap();
    fs::remove_file(&key_file).unwrap();
    // A fresh directory path skips the process key cache.
    let moved = root.path().join("secrets-moved");
    fs::rename(&store.directory, &moved).unwrap();
    let reopened = store_at(moved.clone());
    assert!(reopened.get("account").is_err());
    assert!(fs::read(reopened.record_path("account")).is_ok());
    write_private(&moved.with_extension("kek"), &saved);
    assert_eq!(reopened.get("account").unwrap().as_deref(), Some("secret"));
}

#[test]
fn insecure_or_linked_store_directories_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let insecure = root.path().join("insecure");
    fs::create_dir(&insecure).unwrap();
    fs::set_permissions(&insecure, fs::Permissions::from_mode(0o755)).unwrap();
    let error = store_at(insecure).get("account").unwrap_err();
    assert!(error.contains("0700"), "{error}");

    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    let linked = root.path().join("linked");
    symlink(target, &linked).unwrap();
    let error = store_at(linked).set("account", "secret").unwrap_err();
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
    // Rows: a stored record damaged in one way. Reading it reports that damage
    // as an explicit error that names it. A version byte the store does not
    // know is rejected before any use.
    type Row = (&'static str, fn(&mut Vec<u8>), &'static str);
    let damages: [Row; 3] = [
        (
            "corrupt",
            |bytes: &mut Vec<u8>| {
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
            },
            "authentication",
        ),
        (
            "truncated",
            |bytes: &mut Vec<u8>| *bytes = b"short".to_vec(),
            "truncated",
        ),
        ("version", |bytes: &mut Vec<u8>| bytes[8] = 9, "version"),
    ];
    for (account, damage, message) in damages {
        store.set(account, "secret").expect(account);
        let path = store.record_path(account);
        let mut bytes = fs::read(&path).expect(account);
        damage(&mut bytes);
        write_private(&path, &bytes);
        let error = store.get(account).expect_err(account);
        assert!(error.contains(message), "{account}: {error}");
    }

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
    // Damaged records of one account never block the others.
    store.set("healthy", "still-works").unwrap();
    assert_eq!(
        store.get("healthy").unwrap().as_deref(),
        Some("still-works")
    );
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
