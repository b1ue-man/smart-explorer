//! The session register (VLT-M4, stage `vault_unit`): one unlock for every
//! client of the device, a lock that wipes the key and stops open streams,
//! and the purpose keys of local sealing (C3.6).
use std::cell::Cell;
use std::io::{self, Read, Write};
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::Sha256;

use crate::cryptomator::FormatError;
use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vault::core::locked_site;
use crate::vault::core::registry_types::SiteRecord;
use crate::vault::core::sealed::SealPurpose;
use crate::vfs::{Backend, BackendHandle};

use super::backend_test_support::{pattern, read_with, write_with};
use super::open::{open_location_in, unlocked_session, VaultOpenError, WorkerOpen};
use super::session::{ConsumerKind, LockRequest, RegisterOptions, SessionRegister, SiteState};
use super::session_test_support::{
    local_resolve, settings, site_fixture, test_register, unlocked_site, MemorySecrets,
};
use super::unlock::{unlock_entered, UnlockError, UnlockOptions};

const PASSWORD: &str = "Tresor-Passwort-1";

/// A register that publishes to the process table, as the worker's does.
fn publishing_register() -> Arc<SessionRegister> {
    SessionRegister::new(RegisterOptions {
        publish: true,
        platform: LockPlatform::Desktop,
        background: false,
    })
}

fn assert_locked(error: io::Error, site: &str) {
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert_eq!(locked_site(&error), Some(site), "{error}");
}

fn explicit() -> LockRequest {
    LockRequest::new(LockReason::Explicit, false)
}

#[test]
fn vault_task_session_unlock_once_all_clients() {
    let fixture = site_fixture("Akten", PASSWORD);
    let site = fixture.record.id.clone();
    let (store, resolve) = (&fixture.store, &local_resolve);
    let register = publishing_register();
    let secrets = MemorySecrets::default();
    let before = open_location_in(store, "Akten", "/", resolve, None).err();
    assert_eq!(
        before.as_ref().and_then(VaultOpenError::code),
        Some("vault_locked")
    );

    let first = unlock_entered(
        &register,
        store,
        &secrets,
        "Akten",
        PASSWORD,
        UnlockOptions::default(),
        resolve,
    )
    .expect("unlock");
    assert!(!first.already);
    assert_eq!(first.site, site);
    // Every client of the device opens the same session without a password:
    // by label, by ID prefix, by ID.
    let (by_label, path) =
        open_location_in(store, "Akten", "/Notiz.txt", resolve, None).expect("label");
    write_with(by_label.as_ref(), &path, b"einmal entsperrt");
    let (by_prefix, path) =
        open_location_in(store, &site[..6], "/Notiz.txt", resolve, None).expect("prefix");
    assert_eq!(read_with(by_prefix.as_ref(), &path), b"einmal entsperrt");
    let (by_id, _) = open_location_in(store, &site, "/", resolve, None).expect("id");
    let session = register.session(&site).expect("session").0;
    assert!(Arc::ptr_eq(
        &session,
        &unlocked_session(&site).expect("published")
    ));

    // A process with the session never asks the worker.
    let asked = Cell::new(0);
    let worker: WorkerOpen<'_> =
        &|_: &SiteRecord, _: &str| -> Result<(BackendHandle, String), VaultOpenError> {
            asked.set(asked.get() + 1);
            Err(VaultOpenError::Failed("Background-Worker".into()))
        };
    open_location_in(store, &site, "/", resolve, Some(worker)).expect("own session first");
    assert_eq!(asked.get(), 0);

    // A second unlock derives nothing and replaces nothing, whatever the
    // password; a wrong one is never remembered.
    let again = unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        "falsches-Passwort",
        UnlockOptions::default(),
        resolve,
    )
    .expect("already unlocked");
    assert!(again.already);
    assert_eq!(again.epoch, first.epoch);
    let remember_wrong = UnlockOptions {
        remember: Some(true),
        available: None,
    };
    let refused = unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        "falsches-Passwort",
        remember_wrong,
        resolve,
    );
    assert_eq!(
        refused.unwrap_err(),
        UnlockError::Format(FormatError::WrongPassword)
    );
    assert!(secrets.is_empty());
    assert!(Arc::ptr_eq(
        &session,
        &register.session(&site).expect("same").0
    ));

    // Status and consumers of the register.
    let consumer = register
        .consume(&site, ConsumerKind::Open)
        .expect("consumer");
    let sites = store.load().expect("vault list").sites;
    let status = register.status(&sites);
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].state, SiteState::Unlocked);
    assert_eq!(
        (status[0].epoch, status[0].consumers),
        (Some(first.epoch), 1)
    );
    drop(consumer);
    assert_eq!(register.status(&sites)[0].consumers, 0);

    // One lock ends it for every client.
    assert_eq!(register.lock(&site, explicit()), Ok(true));
    for backend in [&by_label, &by_prefix, &by_id] {
        assert_locked(backend.stat("/Notiz.txt").unwrap_err(), &site);
    }
    assert!(unlocked_session(&site).is_none());
    let after = open_location_in(store, &site, "/", resolve, None)
        .err()
        .expect("a locked site refuses to open");
    assert_eq!(after.code(), Some("vault_locked"));
    assert_eq!(register.status(&sites)[0].state, SiteState::Locked);
    // A process without the session asks the worker.
    let _ = open_location_in(store, &site, "/", resolve, Some(worker));
    assert_eq!(asked.get(), 1);
    let (_, events) = register.events();
    let changes: Vec<_> = events
        .iter()
        .filter(|event| event.site == site)
        .map(|event| (event.state, event.reason))
        .collect();
    let explicit_reason = Some(LockReason::Explicit);
    assert_eq!(
        changes,
        [
            (SiteState::Unlocked, None),
            (SiteState::Locking, explicit_reason),
            (SiteState::Locked, explicit_reason)
        ]
    );
}

#[test]
fn vault_task_session_lock_zeroizes_and_fails_open_handles() {
    let fixture = site_fixture("Belege", PASSWORD);
    let site = fixture.record.id.clone();
    let (store, resolve) = (&fixture.store, &local_resolve);
    let register = publishing_register();
    let secrets = MemorySecrets::default();
    let first = unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        PASSWORD,
        UnlockOptions::default(),
        resolve,
    )
    .expect("unlock");
    let (backend, _) = open_location_in(store, &site, "/", resolve, None).expect("open");
    let big = pattern(70_000);
    write_with(backend.as_ref(), "/gross.bin", &big);
    let mut reader = backend.open_read("/gross.bin").expect("reader");
    let mut head = [0u8; 1000];
    reader.read_exact(&mut head).expect("first bytes");
    assert_eq!(&head[..], &big[..1000]);
    let mut writer = backend.open_write("/neu.txt").expect("writer");
    writer.write_all(b"nicht gespeichert").expect("buffered");
    let session = register.session(&site).expect("session").0;
    // Open streams hold the key cell: no idle time while a file is open.
    assert!(Arc::strong_count(session.keys()) > 1);
    let sealed = register
        .seal_texts(&site, SealPurpose::LogFields, &["/Belege/2026"])
        .expect("sealed while unlocked");

    assert_eq!(register.lock(&site, explicit()), Ok(true));
    // The key is wiped; nothing uses a copy afterwards.
    assert!(session.is_locked() && session.keys().is_wiped());
    assert_locked(session.with_key(|_| Ok(())).unwrap_err(), &site);
    // Open streams fail at once, also with decrypted bytes still buffered,
    // and the writer publishes nothing.
    assert_locked(reader.read(&mut head).unwrap_err(), &site);
    assert_locked(writer.flush().unwrap_err(), &site);
    drop(writer);
    assert_locked(backend.stat("/gross.bin").unwrap_err(), &site);
    // Sealing and the spool wrap key need the key too.
    assert_locked(
        register
            .seal_texts(&site, SealPurpose::LogFields, &["x"])
            .unwrap_err(),
        &site,
    );
    assert_locked(
        register
            .unseal_texts(&site, SealPurpose::LogFields, &sealed)
            .unwrap_err(),
        &site,
    );
    assert_locked(
        register.with_spool_wrap_key(&site, |_| ()).unwrap_err(),
        &site,
    );
    // The bridge keeps the code (`vault_locked`, C7.5).
    #[cfg(unix)]
    {
        let api = crate::mobile::ApiError::from(backend.stat("/").unwrap_err());
        assert_eq!(api.kind, "vault_locked");
    }
    // A new unlock: a new epoch; the old content is there, the unflushed
    // file never appeared.
    let again = unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        PASSWORD,
        UnlockOptions::default(),
        resolve,
    )
    .expect("unlock again");
    assert!(again.epoch > first.epoch);
    let (fresh, _) = open_location_in(store, &site, "/", resolve, None).expect("open again");
    assert_eq!(read_with(fresh.as_ref(), "/gross.bin"), big);
    assert_eq!(
        fresh.stat("/neu.txt").unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(register.lock(&site, explicit()), Ok(true));
}

#[test]
fn vault_task_seal_unseal_purposes_distinct() {
    let register = test_register(LockPlatform::Desktop);
    let (site, session) = unlocked_site(&register, settings(None, true, false, None));
    let (other, _) = unlocked_site(&register, settings(None, true, false, None));
    assert_eq!(SealPurpose::LocalStore.info(), "se-vault-local-store-v1");
    assert_eq!(SealPurpose::LogFields.info(), "se-vault-log-fields-v1");
    assert_eq!(SealPurpose::SpoolWrap.info(), "se-vault-spool-wrap-v1");
    assert!(SealPurpose::LocalStore.over_ipc() && SealPurpose::LogFields.over_ipc());
    assert!(!SealPurpose::SpoolWrap.over_ipc());

    let texts = ["/Akten/2026/Steuer.pdf", "", "Ä/ö/ü"];
    let sealed = register
        .seal_texts(&site, SealPurpose::LocalStore, &texts)
        .expect("sealed");
    for (item, text) in sealed.iter().zip(texts) {
        assert!(item
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
        // The vault content format: 68-byte header, 28 bytes per chunk
        // around the cleartext, an empty text only the header (C1.7).
        let bytes = URL_SAFE_NO_PAD.decode(item).expect("Base64url");
        let chunk = if text.is_empty() { 0 } else { 28 };
        assert_eq!(bytes.len(), 68 + chunk + text.len());
    }
    let opened = register
        .unseal_texts(&site, SealPurpose::LocalStore, &sealed)
        .expect("unsealed");
    let opened: Vec<&str> = opened.iter().map(|text| text.as_str()).collect();
    assert_eq!(opened, texts);
    // A fresh header each time.
    let twice = register
        .seal_texts(&site, SealPurpose::LocalStore, &texts[..1])
        .expect("sealed again");
    assert_ne!(twice[0], sealed[0]);
    // Another purpose, another vault: the tag does not verify.
    for (vault, purpose) in [
        (&site, SealPurpose::LogFields),
        (&other, SealPurpose::LocalStore),
    ] {
        let error = register
            .unseal_texts(vault, purpose, &sealed[..1])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
    }
    // The spool wrap key seals nothing; it is HKDF-SHA256 over encryption
    // key ‖ MAC key with its info, the same for every call, other per vault,
    // and neither half of the masterkey.
    assert!(register
        .with_sealer(&site, SealPurpose::SpoolWrap, |_| Ok(()))
        .is_err());
    let wrap = register
        .with_spool_wrap_key(&site, |key| *key)
        .expect("wrap key");
    assert_eq!(
        wrap,
        register
            .with_spool_wrap_key(&site, |key| *key)
            .expect("again")
    );
    assert_ne!(
        wrap,
        register
            .with_spool_wrap_key(&other, |key| *key)
            .expect("other")
    );
    let (expected, halves) = session
        .with_key(|key| {
            let mut expected = [0u8; 32];
            Hkdf::<Sha256>::new(None, &key.raw()[..])
                .expand(b"se-vault-spool-wrap-v1", &mut expected)
                .map_err(|_| FormatError::Crypto("HKDF"))?;
            Ok((expected, [*key.enc_key(), *key.mac_key()]))
        })
        .expect("key");
    assert_eq!(wrap, expected);
    assert!(!halves.contains(&wrap));
}
