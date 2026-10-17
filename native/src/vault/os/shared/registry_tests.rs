//! The vault list and the storage view `vault-store://` (VLT-M3, stage
//! `vault_unit`).
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};

use crate::vault::core::locator::VaultStoreLocator;
use crate::vault::core::registry_types::{LockSettings, SiteKind, SiteRecord};
use crate::vfs::{
    self, Backend, BackendExtensions, BackendHandle, ChangeSignalMode, LocalBackend, Scheme,
    VfsMeta, VfsResult,
};

use super::backend_test_support::fwd;
use super::open::resolve_store_in;
use super::registry_store::RegistryStore;

const SITE: &str = "0123456789abcdef0123456789abcdef";

fn site(id_digit: char, label: &str, location: &str) -> SiteRecord {
    let id: String = std::iter::repeat_n(id_digit, 32).collect();
    SiteRecord::new(&id, label, location, "jti", 1, LockSettings::desktop())
}

fn store_in(dir: &tempfile::TempDir) -> RegistryStore {
    RegistryStore::at(dir.path().join("vaults"), LockSettings::desktop())
}

#[test]
fn vault_task_registry_roundtrip_unknown_keys_ignored() {
    let record = SiteRecord {
        id: SITE.to_string(),
        label: "Akten".to_string(),
        location: "/home/anna/Tresor".to_string(),
        vault_id: "4f2c1f0e-9b1a-4c3d-8e7f-0a1b2c3d4e5f".to_string(),
        kind: SiteKind::Replica,
        replica_of: Some("fedcba9876543210fedcba9876543210".to_string()),
        seeding: true,
        lock: LockSettings {
            auto_lock_idle_min: Some(30),
            lock_on_sleep: false,
            lock_on_screen_lock: true,
            lock_on_background_min: Some(0),
        },
        remember_password: true,
        created_ms: 1_700_000_000_000,
        extra: Vec::new(),
    };
    let text = record.serialize();
    assert_eq!(
        SiteRecord::parse(&text, LockSettings::desktop()),
        Ok(record.clone())
    );
    // Values the line format cannot hold literally travel as JSON text.
    let mut odd = record.clone();
    odd.label = " Leer zeichen ".to_string();
    odd.location = "/home/anna/Tresor ".to_string();
    let odd_text = odd.serialize();
    assert!(odd_text.contains("label_json=") && odd_text.contains("location_json="));
    assert_eq!(
        SiteRecord::parse(&odd_text, LockSettings::desktop()),
        Ok(odd)
    );
    // Keys of newer versions: ignored, kept, written back.
    let newer = format!("{text}future_key=1\nanother=x=y\n");
    let parsed = SiteRecord::parse(&newer, LockSettings::desktop()).unwrap();
    let extra = [
        ("future_key".to_string(), "1".to_string()),
        ("another".to_string(), "x=y".to_string()),
    ];
    assert_eq!(parsed.extra, extra);
    assert_eq!(
        SiteRecord {
            extra: Vec::new(),
            ..parsed.clone()
        },
        record
    );
    assert!(parsed.serialize().ends_with("future_key=1\nanother=x=y\n"));
    // Missing keys take this platform's defaults; malformed known keys fail.
    let minimal = format!("id={SITE}\nlocation=/v\n");
    let android = SiteRecord::parse(&minimal, LockSettings::android()).unwrap();
    assert_eq!(android.lock, LockSettings::android());
    assert_eq!(android.kind, SiteKind::Original);
    assert!(SiteRecord::parse(
        &format!("{text}seeding=vielleicht\n"),
        LockSettings::desktop()
    )
    .is_err());
    assert!(SiteRecord::parse("id=nicht-hex\nlocation=/v\n", LockSettings::desktop()).is_err());
    assert!(SiteRecord::parse(&format!("id={SITE}\n"), LockSettings::desktop()).is_err());
    // Through the store: unknown keys survive a save of an older reader.
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    store.add(&record).unwrap();
    let file = store.dir().join(format!("{SITE}.conf"));
    std::fs::write(&file, &newer).unwrap();
    let load = store.load().unwrap();
    assert_eq!(load.sites.len(), 1);
    assert_eq!(load.sites[0].extra, extra);
    store.save(&load.sites[0]).unwrap();
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("future_key=1\n"), "{saved}");
    // A broken entry is reported and hides nothing.
    let broken = "fedcba9876543210fedcba9876543210.conf";
    std::fs::write(store.dir().join(broken), "seeding=vielleicht\n").unwrap();
    let load = store.load().unwrap();
    assert_eq!(load.sites.len(), 1);
    assert_eq!(load.broken.len(), 1);
    assert_eq!(load.broken[0].0, broken);
}

#[test]
fn vault_task_registry_duplicate_location_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let first = site('a', "A", "/home/anna/Tresor");
    store.add(&first).unwrap();
    let again = site('b', "B", "/home/anna/Tresor/");
    let refused = store.add(&again).unwrap_err();
    assert!(refused.contains("„A“"), "{refused}");
    // Windows spellings of one folder are one location.
    store.add(&site('c', "C", r"C:\Tresore\Akten")).unwrap();
    assert!(store.add(&site('d', "D", "c:/tresore/akten/")).is_err());
    // One ID once.
    let mut same_id = site('a', "A2", "/home/anna/Anderer");
    assert!(store.add(&same_id).is_err());
    same_id.id = "b".repeat(32);
    store.add(&same_id).unwrap();
    assert_eq!(store.load().unwrap().sites.len(), 3);
    // `<SITE>`: ID, unique label, ID prefix of at least 4 hex digits.
    let id_a = "a".repeat(32);
    assert_eq!(store.resolve(&id_a).unwrap().id, id_a);
    assert_eq!(store.resolve(&id_a.to_uppercase()).unwrap().id, id_a);
    assert_eq!(store.resolve("A").unwrap().id, id_a);
    assert_eq!(store.resolve("aaaa").unwrap().id, id_a);
    assert!(store.resolve("aaa").is_err());
    assert!(store.resolve("unbekannt").is_err());
    // A save cannot move a site onto another site's location.
    let mut moved = store.get(&"b".repeat(32)).unwrap().unwrap();
    moved.location = "/home/anna/Tresor".to_string();
    assert!(store.save(&moved).is_err());
    // No vault in its own storage view or in itself.
    assert!(store
        .add(&site('e', "E", &format!("vault-store://{id_a}")))
        .is_err());
    let id_e = "e".repeat(32);
    assert!(store
        .add(&site('e', "E", &format!("vault://{id_e}/x")))
        .is_err());
    // Removal keeps every other entry.
    assert_eq!(store.remove(&"b".repeat(32)), Ok(true));
    assert_eq!(store.remove(&"b".repeat(32)), Ok(false));
    assert_eq!(store.load().unwrap().sites.len(), 2);
}

/// A storage with content hashes and a poll change signal.
struct Signalled(LocalBackend);

impl Backend for Signalled {
    fn scheme(&self) -> Scheme {
        Scheme::Webdav
    }
    fn root_display(&self) -> String {
        self.0.root_display()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.0.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.0.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.0.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.0.open_write(path)
    }
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.0.rename(src, dst)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.0.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.0.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.0.mkdir_all(path)
    }
    fn provides_content_hash(&self) -> bool {
        true
    }
}

impl BackendExtensions for Signalled {
    fn change_signal_mode(&self, _root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        Ok(Some(ChangeSignalMode::Poll))
    }
}

#[test]
fn vault_task_vault_store_resolves_to_inner_with_hash_and_signal() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    store
        .add(&SiteRecord::new(
            SITE,
            "Akten",
            "webdav://anna@cloud:443/dav/Tresor",
            "jti",
            1,
            LockSettings::desktop(),
        ))
        .unwrap();
    let storage: BackendHandle = Arc::new(Signalled(LocalBackend::new("/")));
    let seen = Mutex::new(Vec::new());
    let resolve = |location: &str| -> Result<(BackendHandle, String), String> {
        seen.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(location.to_string());
        Ok((Arc::clone(&storage), "/dav/Tresor".to_string()))
    };
    let locator = VaultStoreLocator::parse(&format!("vault-store://{SITE}")).unwrap();
    let (backend, root) = resolve_store_in(&store, &locator.site, &resolve).unwrap();
    // No wrapper (C2.3): the storage itself at the vault root.
    assert!(Arc::ptr_eq(&backend, &storage));
    assert_eq!(root, "/dav/Tresor");
    assert_eq!(
        *seen.lock().unwrap_or_else(PoisonError::into_inner),
        ["webdav://anna@cloud:443/dav/Tresor"]
    );
    assert!(backend.provides_content_hash());
    assert_eq!(
        vfs::change_signal_mode(&*backend, &root).unwrap(),
        Some(ChangeSignalMode::Poll)
    );
    assert_eq!(backend.state_identity(), storage.state_identity());
    assert_eq!(backend.namespace_identity(), storage.namespace_identity());
    // Only the full site ID names a storage view.
    assert!(resolve_store_in(&store, &SITE[..8], &resolve).is_err());
    assert!(resolve_store_in(&store, "Akten", &resolve).is_err());
    assert!(resolve_store_in(&store, &"f".repeat(32), &resolve).is_err());
}

#[test]
fn vault_task_vault_store_overlap_with_inner_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(&dir);
    let vault_root = dir.path().join("Tresor");
    std::fs::create_dir_all(vault_root.join("d")).unwrap();
    let other = dir.path().join("Anderes");
    std::fs::create_dir_all(&other).unwrap();
    store
        .add(&SiteRecord::new(
            SITE,
            "Akten",
            &fwd(&vault_root),
            "jti",
            1,
            LockSettings::desktop(),
        ))
        .unwrap();
    let resolve = |location: &str| -> Result<(BackendHandle, String), String> {
        Ok((Arc::new(LocalBackend::new("/")), location.to_string()))
    };
    let (storage, root) = resolve_store_in(&store, SITE, &resolve).unwrap();
    assert_eq!(root, fwd(&vault_root));
    let local = LocalBackend::new("/");
    // The storage view overlaps its own vault folder, inside and above it.
    let inside = fwd(&vault_root.join("d"));
    assert!(vfs::validate_sync_roots(&*storage, &root, &local, &inside).is_err());
    assert!(vfs::validate_sync_roots(&*storage, &root, &local, &root).is_err());
    assert!(vfs::validate_sync_roots(&local, &fwd(dir.path()), &*storage, &root).is_err());
    // A separate folder is fine.
    assert!(vfs::validate_sync_roots(&*storage, &root, &local, &fwd(&other)).is_ok());
}
