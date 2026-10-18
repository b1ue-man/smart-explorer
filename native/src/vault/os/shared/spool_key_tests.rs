//! Signals of the spool keys and sealed journal fields of vault drives
//! (VLT-M12, stage `vault_unit`, job linux-core: unit; C3.6, P1): the
//! wrapped key survives a restart and a password change, exists only for an
//! unlocked vault, the two derived keys differ from each other and from the
//! spool key, and the journal sealer round-trips paths and conflict texts.
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::cryptomator::decrypt_bytes;
use crate::mount::PathSealer;
use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vault::core::locked_site;

use super::journal_sealer::{sealed_mount_storage, JournalPathSealer, MAX_FIELD};
use super::mount_bridge::VaultDriveBridge;
use super::password::change_site_password;
use super::registry_store::RegistryStore;
use super::session::{LockRequest, SessionRegister};
use super::session_test_support::{
    local_resolve, site_fixture, test_register, MemorySecrets, SiteFixture,
};
use super::spool_key::{
    load_or_create, unwrap, wrap, SpoolKey, CONTENT_INFO, SPOOL_KEY_FILE, TEXT_INFO, WRAPPED_LEN,
};
use super::unlock::{unlock_entered, UnlockOptions};

const PASSWORD: &str = "Laufwerk-Passwort-1";
const NEW_PASSWORD: &str = "Laufwerk-Passwort-2";

pub(super) fn bridge_for(
    fixture: &SiteFixture,
    register: Arc<SessionRegister>,
) -> Arc<VaultDriveBridge> {
    let store = RegistryStore::at(fixture.store.dir().to_path_buf(), fixture.store.defaults());
    let thumbnails = fixture.dir.path().join("thumbnails");
    VaultDriveBridge::new(register, store, Box::new(local_resolve), Some(thumbnails))
}

pub(super) fn unlock(register: &SessionRegister, fixture: &SiteFixture, password: &str) {
    let options = UnlockOptions {
        remember: None,
        available: None,
    };
    unlock_entered(
        register,
        &fixture.store,
        &MemorySecrets::default(),
        &fixture.record.id,
        password,
        options,
        &local_resolve,
    )
    .expect("vault unlocked");
}

fn lock(register: &SessionRegister, site: &str) {
    let request = LockRequest::new(LockReason::Explicit, false);
    assert_eq!(register.lock(site, request), Ok(true));
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn mount_dir(base: &Path) -> std::path::PathBuf {
    let dir = base.join("mount-cache").join("drive-1");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn vault_task_spool_key_wrap_unwrap_after_restart() {
    let base = tempfile::tempdir().unwrap();
    let dir = mount_dir(base.path());
    let wrap_key = [7u8; 32];
    let first = load_or_create(&dir, &wrap_key).unwrap();
    let stored = fs::read(dir.join(SPOOL_KEY_FILE)).unwrap();
    assert_eq!(stored.len(), WRAPPED_LEN);
    assert!(
        !contains(&stored, first.expose()),
        "the spool key lies in clear"
    );
    // AES-KW is deterministic: the file holds exactly the wrapped key.
    let wrapped = wrap(&wrap_key, &first).unwrap();
    assert_eq!(&stored[..], &wrapped[..]);
    assert_eq!(
        unwrap(&wrap_key, &wrapped).unwrap().expose(),
        first.expose()
    );

    // After a restart the worker unwraps the same key; the file stays.
    let again = load_or_create(&dir, &wrap_key).unwrap();
    assert_eq!(again.expose(), first.expose());
    assert_eq!(fs::read(dir.join(SPOOL_KEY_FILE)).unwrap(), stored);

    // Another vault's wrap key yields no key and never replaces the file.
    let error = load_or_create(&dir, &[8u8; 32]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(dir.join(SPOOL_KEY_FILE)).unwrap(), stored);
    // Changed bytes yield no key either.
    let mut changed = wrapped;
    changed[5] ^= 0x01;
    assert!(unwrap(&wrap_key, &changed).is_err());
    // A file of another length is refused.
    fs::write(dir.join(SPOOL_KEY_FILE), &stored[..39]).unwrap();
    assert!(load_or_create(&dir, &wrap_key).is_err());

    // Two drives get two keys.
    let other_dir = base.path().join("mount-cache").join("drive-2");
    fs::create_dir_all(&other_dir).unwrap();
    let other = load_or_create(&other_dir, &wrap_key).unwrap();
    assert_ne!(other.expose(), first.expose());
}

#[test]
fn vault_task_spool_key_stable_over_password_change() {
    let fixture = site_fixture("Laufwerkstresor", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));
    let dir = mount_dir(fixture.dir.path());
    unlock(&register, &fixture, PASSWORD);
    let before = bridge.spool_key(&site, &dir).unwrap();
    let stored = fs::read(dir.join(SPOOL_KEY_FILE)).unwrap();
    lock(&register, &site);

    change_site_password(
        &fixture.store,
        &MemorySecrets::default(),
        &site,
        PASSWORD,
        NEW_PASSWORD,
        None,
        &local_resolve,
    )
    .expect("password changed");
    unlock(&register, &fixture, NEW_PASSWORD);
    let after = bridge.spool_key(&site, &dir).unwrap();
    assert_eq!(after.expose(), before.expose());
    assert_eq!(fs::read(dir.join(SPOOL_KEY_FILE)).unwrap(), stored);
}

#[test]
fn vault_task_spool_key_only_unlocked() {
    let fixture = site_fixture("Gesperrt", PASSWORD);
    let site = fixture.record.id.clone();
    let register = test_register(LockPlatform::Desktop);
    let bridge = bridge_for(&fixture, Arc::clone(&register));
    let dir = mount_dir(fixture.dir.path());

    // Locked: no key, no key file, the error is `vault_locked`.
    let error = bridge.spool_key(&site, &dir).unwrap_err();
    assert_eq!(locked_site(&error), Some(site.as_str()));
    assert!(!dir.join(SPOOL_KEY_FILE).exists());

    unlock(&register, &fixture, PASSWORD);
    let key = bridge.spool_key(&site, &dir).unwrap();
    assert!(dir.join(SPOOL_KEY_FILE).exists());

    // After the lock the stored key is not reachable any more.
    lock(&register, &site);
    let error = bridge.spool_key(&site, &dir).unwrap_err();
    assert_eq!(locked_site(&error), Some(site.as_str()));
    // An unknown site is locked as well (it has no session).
    let unknown = bridge.spool_key("0123456789abcdef0123456789abcdef", &dir);
    assert!(unknown.is_err());
    drop(key);
}

#[test]
fn vault_task_journal_text_key_derived_from_spool_key() {
    assert_eq!(CONTENT_INFO, "se-mount-spool-content-v1");
    assert_eq!(TEXT_INFO, "se-mount-journal-v1");
    let raw = [0x42u8; 32];
    let key = SpoolKey::from_bytes(&raw).unwrap();
    let content = key.derive(CONTENT_INFO).unwrap();
    let text = key.derive(TEXT_INFO).unwrap();
    // HKDF-SHA256 without salt over the spool key, info of the purpose.
    for (info, derived) in [(CONTENT_INFO, &content), (TEXT_INFO, &text)] {
        let mut expected = [0u8; 64];
        Hkdf::<Sha256>::new(None, &raw)
            .expand(info.as_bytes(), &mut expected)
            .unwrap();
        assert_eq!(&derived[..], &expected[..], "{info}");
    }
    // Text key ≠ content key ≠ spool key.
    assert_ne!(&text[..32], &content[..32]);
    assert_ne!(&text[..32], &raw[..]);
    assert_ne!(&content[..32], &raw[..]);
    assert_eq!(&key.derive(TEXT_INFO).unwrap()[..], &text[..]);

    // A field sealed under the text key does not open under the content key.
    let sealer = JournalPathSealer::new(&key).unwrap();
    let sealed = URL_SAFE_NO_PAD
        .decode(sealer.seal("/Tresor/Bericht.txt").unwrap())
        .unwrap();
    assert!(decrypt_bytes(&key.content_cryptor().unwrap(), &sealed, 1024).is_err());
    let opened = decrypt_bytes(&key.text_cryptor().unwrap(), &sealed, 1024).unwrap();
    assert_eq!(&opened[..], b"/Tresor/Bericht.txt");
    // Another spool key derives other keys.
    let other = SpoolKey::from_bytes(&[0x43u8; 32]).unwrap();
    assert_ne!(&other.derive(TEXT_INFO).unwrap()[..], &text[..]);
    assert!(SpoolKey::from_bytes(&[1u8; 31]).is_err());
}

#[test]
fn vault_task_journal_sealer_roundtrip() {
    let key = SpoolKey::generate().unwrap();
    let sealer = JournalPathSealer::new(&key).unwrap();
    for path in [
        "/",
        "/Bilder/Urlaub ä.jpg",
        "/Bericht.pdf.se-mount-0000000000000001",
        "/x/Bericht.pdf.se-mount-delete-0000000000000007",
    ] {
        let sealed = sealer.seal(path).unwrap();
        assert!(sealed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
        let name = path.rsplit('/').next().unwrap_or_default();
        assert!(
            name.is_empty() || !sealed.contains(name),
            "{path} stayed visible"
        );
        assert_eq!(sealer.unseal(&sealed).unwrap(), path);
        // Every seal has its own header: equal paths give different texts.
        assert_ne!(sealer.seal(path).unwrap(), sealed);
    }
    // Conflict texts are sealed as well.
    let detail = "Lokale Wiederherstellungsdatei fuer /Geheim/Plan.txt fehlt";
    let sealed_detail = sealer.seal_detail(detail).unwrap();
    assert!(!sealed_detail.contains("Geheim"));
    assert_eq!(sealer.unseal_detail(&sealed_detail).unwrap(), detail);
    // A long conflict text is cut, never refused (a journal write must not
    // fail over it); a path that long is refused.
    let long = "x".repeat(MAX_FIELD + 100);
    let cut = sealer.unseal_detail(&sealer.seal_detail(&long).unwrap());
    assert_eq!(cut.unwrap().len(), MAX_FIELD);
    assert!(sealer.seal(&long).is_err());

    // Another key, a changed text and a plain text open nothing.
    let sealed = sealer.seal("/Geheim/Plan.txt").unwrap();
    let other = JournalPathSealer::new(&SpoolKey::generate().unwrap()).unwrap();
    assert_eq!(
        other.unseal(&sealed).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let middle = sealed.len() / 2;
    let swapped = if sealed.as_bytes()[middle] == b'A' {
        'B'
    } else {
        'A'
    };
    let mut changed = sealed.clone();
    changed.replace_range(middle..middle + 1, &swapped.to_string());
    assert!(sealer.unseal(&changed).is_err());
    assert!(sealer.unseal("/Geheim/Plan.txt").is_err());

    // The storage of a vault drive pairs the sealer with the sealed spool.
    let storage = sealed_mount_storage(&key).unwrap();
    let field = storage.paths.seal("/z").unwrap();
    assert_eq!(sealer.unseal(&field).unwrap(), "/z");
}
