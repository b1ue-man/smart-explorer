//! Porting without secret access, locked protector, foreign user, overwrite
//! of old bytes and parallel processes (SP1 of the "Lücken schließen" batch).
use super::{contains, fixture, store_at, write_private, KeyFileOnly};
use crate::creds::key_protector::{KeyProtector, Protected, APP_KEY_FILE};
use crate::creds::sealed_record::{encode_legacy, DataKey, KeyringDescriptor};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use super::super::{fs as store_fs, keys, migrate, FileStore};

/// A protector whose key recovery can be switched off, like a failing
/// `systemd-creds` or a missing Android Keystore key. Counts new keys.
struct Switchable {
    inner: KeyFileOnly,
    locked: Arc<AtomicBool>,
    new_keys: Arc<AtomicUsize>,
}

impl KeyProtector for Switchable {
    fn new_key(&self) -> Result<DataKey, String> {
        self.new_keys.fetch_add(1, Ordering::SeqCst);
        self.inner.new_key()
    }

    fn protect(&self, directory: &Path, key: &DataKey) -> Result<Protected, String> {
        self.inner.protect(directory, key)
    }

    fn recover(&self, directory: &Path, descriptor: &KeyringDescriptor) -> Result<DataKey, String> {
        if self.locked.load(Ordering::SeqCst) {
            return Err("Geheimnisspeicher gesperrt: Test-Hüter liefert keinen Schlüssel".into());
        }
        self.inner.recover(directory, descriptor)
    }
}

struct Switch {
    locked: Arc<AtomicBool>,
    new_keys: Arc<AtomicUsize>,
}

impl Switch {
    fn lock(&self, locked: bool) {
        self.locked.store(locked, Ordering::SeqCst);
    }

    fn new_keys(&self) -> usize {
        self.new_keys.load(Ordering::SeqCst)
    }
}

fn switchable_fixture() -> (tempfile::TempDir, FileStore, Switch) {
    let root = tempfile::tempdir().unwrap();
    let locked = Arc::new(AtomicBool::new(false));
    let new_keys = Arc::new(AtomicUsize::new(0));
    let protector = Switchable {
        inner: KeyFileOnly,
        locked: locked.clone(),
        new_keys: new_keys.clone(),
    };
    let store = FileStore::with_protector(root.path().join("secrets-v1"), Arc::new(protector));
    (root, store, Switch { locked, new_keys })
}

/// Pretends to be another effective user for the current thread.
struct ForeignUser;

impl ForeignUser {
    fn new() -> Self {
        let other = unsafe { libc::geteuid() } + 1;
        store_fs::UID_OVERRIDE.with(|forced| forced.set(Some(other)));
        ForeignUser
    }
}

impl Drop for ForeignUser {
    fn drop(&mut self) {
        store_fs::UID_OVERRIDE.with(|forced| forced.set(None));
    }
}

fn make_directory(store: &FileStore) {
    fs::create_dir(&store.directory).unwrap();
    fs::set_permissions(&store.directory, fs::Permissions::from_mode(0o700)).unwrap();
}

fn write_legacy(store: &FileStore, account: &str, secret: &str) -> Vec<u8> {
    let record = encode_legacy(account, secret);
    write_private(&store.record_path(account), &record);
    record
}

fn directory_contains(directory: &Path, needle: &[u8]) -> bool {
    fs::read_dir(directory).unwrap().any(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap_or_default();
        contains(&bytes, needle)
    })
}

/// As a new process would start: no cached key, nothing ported yet.
fn restart(store: &FileStore) {
    keys::forget(&store.directory);
    migrate::forget_ported(&store.directory);
}

fn read_through(held: &fs::File) -> Vec<u8> {
    let mut reader = held;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    bytes
}

#[test]
fn luecken_task_port_without_secret_access_seals_every_old_secret() {
    let (root, store) = fixture();
    make_directory(&store);
    let identity_secret = "cd".repeat(32);
    write_legacy(&store, "share:identity:iroh_secret", &identity_secret);
    write_legacy(&store, "sftp://u@h:22/", "old-password");
    let stage = store.directory.join(".stage-7-ab");
    write_private(&stage, &encode_legacy("half-written", "stage-secret"));

    // No get, no set: the port alone must do it.
    store.port_existing().unwrap();

    let old_secrets: [&[u8]; 3] = [identity_secret.as_bytes(), b"old-password", b"stage-secret"];
    for needle in old_secrets {
        assert!(!directory_contains(&store.directory, needle));
    }
    assert!(!stage.exists());
    for account in ["share:identity:iroh_secret", "sftp://u@h:22/"] {
        let record = fs::read(store.record_path(account)).unwrap();
        assert!(record.starts_with(b"SESEC02\0"), "{account}");
    }
    let status = store.status().unwrap();
    assert_eq!(status.plaintext_records, 0);
    assert_eq!(status.stage_files, 0);
    assert_eq!(status.unreadable_records, 0);
    assert_eq!(status.protector.as_deref(), Some(APP_KEY_FILE));
    assert!(status.locked.is_none());
    let key_file = fs::metadata(root.path().join("secrets-v1.kek")).unwrap();
    assert_eq!(key_file.mode() & 0o7777, 0o600);
    assert_eq!(key_file.len(), 32);
    // The ported secrets read back unchanged.
    assert_eq!(
        store.get("share:identity:iroh_secret").unwrap(),
        Some(identity_secret)
    );
    assert_eq!(
        store.get("sftp://u@h:22/").unwrap().as_deref(),
        Some("old-password")
    );
}

#[test]
fn luecken_task_port_without_plaintext_creates_no_key_and_no_directory() {
    let (root, store) = fixture();
    // No store at all: nothing is created.
    store.port_existing().unwrap();
    assert!(!store.directory.exists());
    assert!(!root.path().join("secrets-v1.kek").exists());

    // An empty store: no key, no key file.
    make_directory(&store);
    store.port_existing().unwrap();
    assert!(!store.directory.join("keyring-v2").exists());
    assert!(!root.path().join("secrets-v1.kek").exists());

    // Only a leftover stage file: it goes, still no key.
    restart(&store);
    let stage = store.directory.join(".stage-1-0011");
    write_private(&stage, &encode_legacy("half", "stage-only-secret"));
    store.port_existing().unwrap();
    assert!(!stage.exists());
    assert!(!store.directory.join("keyring-v2").exists());
    assert!(!root.path().join("secrets-v1.kek").exists());
}

#[test]
fn luecken_task_port_leaves_a_sealed_store_byte_for_byte_alone() {
    let (_root, store) = fixture();
    store.set("account", "sealed-secret").unwrap();
    let record = fs::read(store.record_path("account")).unwrap();
    let descriptor = fs::read(store.directory.join("keyring-v2")).unwrap();
    restart(&store);
    store.port_existing().unwrap();
    assert_eq!(fs::read(store.record_path("account")).unwrap(), record);
    assert_eq!(
        fs::read(store.directory.join("keyring-v2")).unwrap(),
        descriptor
    );
}

#[test]
fn luecken_task_locked_protector_keeps_plaintext_and_makes_no_new_key() {
    let (_root, store, switch) = switchable_fixture();
    // The identity secret of the Share host: an error here must stay an
    // error, because `ShareIdentity` refuses to start without it instead of
    // creating a new identity.
    let account = "share:identity:iroh_secret";
    store.set(account, "sealed-identity-secret").unwrap();
    assert_eq!(switch.new_keys(), 1);
    let record = fs::read(store.record_path(account)).unwrap();
    let descriptor = fs::read(store.directory.join("keyring-v2")).unwrap();
    // An older version writes plaintext afterwards.
    let late = write_legacy(&store, "late", "late-plain-secret");

    restart(&store);
    switch.lock(true);
    assert!(store.port_existing().is_err());
    assert_eq!(fs::read(store.record_path("late")).unwrap(), late);
    assert!(store.get(account).unwrap_err().contains("gesperrt"));
    assert!(store.set(account, "overwrite").is_err());
    assert!(store.set("other", "x").is_err());
    assert_eq!(fs::read(store.record_path(account)).unwrap(), record);
    assert_eq!(
        fs::read(store.directory.join("keyring-v2")).unwrap(),
        descriptor
    );
    assert!(!store.record_path("other").exists());
    assert_eq!(
        switch.new_keys(),
        1,
        "a locked store must not make a new key"
    );
    let status = store.status().unwrap();
    assert!(status.locked.is_some());
    assert_eq!(status.plaintext_records, 1);
    // Reading plaintext needs no key.
    assert_eq!(
        store.get("late").unwrap().as_deref(),
        Some("late-plain-secret")
    );

    // The failure was not remembered: the next access ports.
    switch.lock(false);
    assert_eq!(
        store.get(account).unwrap().as_deref(),
        Some("sealed-identity-secret")
    );
    let ported = fs::read(store.record_path("late")).unwrap();
    assert!(ported.starts_with(b"SESEC02\0"));
    assert!(!contains(&ported, b"late-plain-secret"));
    assert_eq!(store.status().unwrap().plaintext_records, 0);
    assert_eq!(switch.new_keys(), 1);
}

#[test]
fn luecken_task_stage_files_go_even_when_the_protector_is_locked() {
    let (_root, store, switch) = switchable_fixture();
    store.set("account", "sealed").unwrap();
    restart(&store);
    switch.lock(true);
    let stage = store.directory.join(".stage-9-aa");
    write_private(&stage, &encode_legacy("half", "stage-plain-secret"));
    let held = fs::File::open(&stage).unwrap();

    store.port_existing().unwrap();

    assert!(!stage.exists());
    let old = read_through(&held);
    assert!(!old.is_empty());
    assert!(
        old.iter().all(|byte| *byte == 0),
        "stage bytes were not overwritten"
    );
    assert_eq!(switch.new_keys(), 1);
}

#[test]
fn luecken_task_a_failed_port_is_retried_and_never_marked_done() {
    let (_root, store) = fixture();
    make_directory(&store);
    write_legacy(&store, "good", "good-secret");
    let mut damaged = write_legacy(&store, "damaged", "damaged-secret");
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    write_private(&store.record_path("damaged"), &damaged);

    assert!(store.port_existing().is_err());
    // The damaged record stays as it is; the good one was sealed.
    assert_eq!(fs::read(store.record_path("damaged")).unwrap(), damaged);
    assert!(fs::read(store.record_path("good"))
        .unwrap()
        .starts_with(b"SESEC02\0"));
    assert!(
        store.port_existing().is_err(),
        "a failed port must be retried"
    );

    // Repaired by hand: the next port succeeds.
    write_legacy(&store, "damaged", "damaged-secret");
    store.port_existing().unwrap();
    assert!(fs::read(store.record_path("damaged"))
        .unwrap()
        .starts_with(b"SESEC02\0"));
    assert_eq!(
        store.get("damaged").unwrap().as_deref(),
        Some("damaged-secret")
    );
}

#[test]
fn luecken_task_foreign_effective_user_creates_and_ports_nothing() {
    let (root, store) = fixture();
    make_directory(&store);
    let record = write_legacy(&store, "account", "plain-secret");
    {
        let _foreign = ForeignUser::new();
        assert!(store.port_existing().is_err());
        assert!(store.get("account").is_err());
        assert!(store.set("account", "new").is_err());
        assert!(store.delete("account").is_err());
        assert!(store.status().is_err());
    }
    assert!(!store.directory.join("keyring-v2").exists());
    assert!(!store.directory.join("store.lock").exists());
    assert!(!root.path().join("secrets-v1.kek").exists());
    assert_eq!(fs::read(store.record_path("account")).unwrap(), record);
}

#[test]
fn luecken_task_store_is_never_created_inside_a_directory_of_another_user() {
    let (root, store) = fixture();
    {
        let _foreign = ForeignUser::new();
        let error = store.set("account", "secret").unwrap_err();
        assert!(error.contains("another user"), "{error}");
        assert!(store.get("account").is_err());
    }
    assert!(!store.directory.exists());
    assert!(!root.path().join("secrets-v1.kek").exists());
}

#[test]
fn luecken_task_replacing_a_late_plaintext_record_overwrites_its_old_bytes() {
    let (_root, store) = fixture();
    store.set("first", "sealed").unwrap();
    write_legacy(&store, "late", "old-plain-secret");
    let held = fs::File::open(store.record_path("late")).unwrap();

    store.set("late", "new-secret").unwrap();

    let old = read_through(&held);
    assert!(!old.is_empty());
    assert!(old.iter().all(|byte| *byte == 0), "old plaintext survived");
    assert_eq!(store.get("late").unwrap().as_deref(), Some("new-secret"));
}

#[test]
fn luecken_task_delete_overwrites_the_removed_plaintext_record() {
    let (_root, store) = fixture();
    store.set("first", "sealed").unwrap();
    write_legacy(&store, "late", "plain-to-delete");
    let held = fs::File::open(store.record_path("late")).unwrap();

    store.delete("late").unwrap();

    assert!(!store.record_path("late").exists());
    let old = read_through(&held);
    assert!(!old.is_empty());
    assert!(
        old.iter().all(|byte| *byte == 0),
        "deleted plaintext survived"
    );
    assert_eq!(store.get("late").unwrap(), None);
}

#[test]
fn luecken_task_unreadable_plaintext_record_is_reported_not_hidden() {
    let (_root, store) = fixture();
    make_directory(&store);
    let record = write_legacy(&store, "account", "plain-secret");
    fs::set_permissions(
        store.record_path("account"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let stage = store.directory.join(".stage-3-cc");
    write_private(&stage, &encode_legacy("half", "stage-secret"));

    let status = store.status().unwrap();
    assert_eq!(status.plaintext_records, 0);
    assert_eq!(status.unreadable_records, 1);
    assert_eq!(status.stage_files, 1);

    // The port never touches a record it cannot vouch for.
    store.port_existing().unwrap();
    assert_eq!(fs::read(store.record_path("account")).unwrap(), record);
    let status = store.status().unwrap();
    assert_eq!(status.unreadable_records, 1);
    assert_eq!(status.stage_files, 0);
}

#[test]
fn luecken_task_parallel_processes_port_every_record_exactly_once() {
    let (_root, store) = fixture();
    make_directory(&store);
    for index in 0..24 {
        write_legacy(&store, &format!("acct-{index}"), &format!("secret-{index}"));
    }
    let directory = store.directory.clone();
    let barrier = Arc::new(Barrier::new(6));
    let workers: Vec<_> = (0..6)
        .map(|worker| {
            let directory = directory.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let store = store_at(directory);
                barrier.wait();
                // Each round behaves like a freshly started process.
                for round in 0..4 {
                    migrate::forget_ported(&store.directory);
                    if (worker + round) % 2 == 0 {
                        store.port_existing().unwrap();
                    } else {
                        store.get(&format!("acct-{}", worker + round)).unwrap();
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }

    let mut key_ids = HashSet::new();
    for index in 0..24 {
        let account = format!("acct-{index}");
        let record = fs::read(store.record_path(&account)).unwrap();
        assert!(record.starts_with(b"SESEC02\0"), "{account}");
        // Magic (8), version (1), account digest (32), then the key id.
        key_ids.insert(record[41..57].to_vec());
        assert_eq!(
            store.get(&account).unwrap(),
            Some(format!("secret-{index}"))
        );
    }
    assert_eq!(key_ids.len(), 1, "exactly one data key must exist");
    assert!(!directory_contains(&store.directory, b"secret-"));
    assert!(fs::read_dir(&store.directory).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".stage-")));
}

#[test]
fn luecken_task_key_file_appears_complete_and_ignores_stale_temporary_files() {
    let (root, store) = fixture();
    // A leftover of an interrupted earlier attempt never blocks a new key.
    write_private(&root.path().join("secrets-v1.kek.tmp-1-aa"), b"");
    store.set("account", "secret").unwrap();
    let key_file = fs::metadata(root.path().join("secrets-v1.kek")).unwrap();
    assert_eq!(key_file.len(), 32);
    assert_eq!(key_file.mode() & 0o7777, 0o600);
    let ours = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.contains(".kek.tmp-") && name != "secrets-v1.kek.tmp-1-aa"
        })
        .count();
    assert_eq!(ours, 0, "the staged key file must be renamed away");
    assert_eq!(store.get("account").unwrap().as_deref(), Some("secret"));
}
