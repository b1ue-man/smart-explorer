//! Management of vault sites (VLT-M5, stage `vault_unit`): detection
//! messages, removal, the password rule, recovery keys and remembered
//! passwords (C3.5, C7.1).
use std::cell::RefCell;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

use crate::cryptomator::{FormatError, Masterkey, MasterkeyFile};
use crate::vault::core::lock_policy::{LockPlatform, LockReason};
use crate::vfs::LocalBackend;

use super::backend_test_support::fwd;
use super::manage::{
    add_site, create_site, create_vault, detect_storage, jobs_using, recovery_key_info,
    remove_site, update_settings, ManageError, SettingsChange, CREATE_RECOVERY_HINT,
};
use super::password::{change_password, check_new_password, PasswordError};
use super::registry_store::RegistryStore;
use super::remember::{account, SecretStore};
use super::session::{LockRequest, RegisterOptions, SessionRegister};
use super::session_test_support::{local_resolve, site_fixture, MemorySecrets};
use super::unlock::{unlock_entered, unlock_saved, unlock_storage, UnlockError, UnlockOptions};

const PASSWORD: &str = "Tresor-Passwort-1";

fn token(header: &str, claims: &str) -> String {
    format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(claims),
        URL_SAFE_NO_PAD.encode("keine-echte-Signatur")
    )
}

fn claims(format: i64) -> String {
    format!(
        r#"{{"jti":"6d2a0c1e-hub-or-not","format":{format},"cipherCombo":"SIV_GCM","shorteningThreshold":220}}"#
    )
}

fn folder(base: &Path, name: &str) -> String {
    let root = format!("{}/{name}", fwd(base));
    std::fs::create_dir_all(&root).expect("folder");
    root
}

#[test]
fn vault_task_detect_hub_and_format7_messages() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let backend = LocalBackend::new("/");
    let store = RegistryStore::at(
        dir.path().join("vaults"),
        crate::vault::LockSettings::desktop(),
    );
    let hub_text = "Cryptomator-Hub-Tresore werden nicht unterstützt";
    let format7_text = "Tresorformat 7 nicht unterstützt: den Tresor einmal in Cryptomator \
                        öffnen, Cryptomator aktualisiert ihn auf Format 8";

    // A Hub vault: recognized, named, refused before any key is loaded.
    let hub = folder(dir.path(), "hub");
    let header = r#"{"kid":"hub+https://hub.example/vaults/1/","typ":"JWT","alg":"ES384"}"#;
    std::fs::write(
        format!("{hub}/vault.cryptomator"),
        token(header, &claims(8)),
    )
    .unwrap();
    let detection = detect_storage(&backend, &hub).expect("detect");
    assert!(detection.is_vault);
    assert_eq!(detection.key_source, Some("hub"));
    assert_eq!(detection.format, Some(8));
    assert_eq!(detection.vault_id.as_deref(), Some("6d2a0c1e-hub-or-not"));
    assert_eq!(detection.problem.as_deref(), Some(hub_text));
    let error = add_site(&store, &hub, None, &local_resolve).unwrap_err();
    assert_eq!(error, ManageError::Refused(hub_text.into()));
    let error = unlock_storage(&backend, &hub, PASSWORD, None).unwrap_err();
    assert_eq!(error, UnlockError::Format(FormatError::HubVault));
    assert_eq!(
        (error.code(), error.to_string().as_str()),
        (Some("vault_hub"), hub_text)
    );

    // Format 7: only a key file with version 7, no vault.cryptomator.
    let seven = folder(dir.path(), "format7");
    let key = Masterkey::generate().unwrap();
    let bytes = MasterkeyFile::create(&key, PASSWORD, None).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["version"] = serde_json::json!(7);
    std::fs::write(format!("{seven}/masterkey.cryptomator"), json.to_string()).unwrap();
    let detection = detect_storage(&backend, &seven).expect("detect");
    assert!(detection.is_vault);
    assert_eq!(detection.format, Some(7));
    assert_eq!(detection.problem.as_deref(), Some(format7_text));
    let error = add_site(&store, &seven, None, &local_resolve).unwrap_err();
    assert_eq!(error, ManageError::Refused(format7_text.into()));
    let error = unlock_storage(&backend, &seven, PASSWORD, None).unwrap_err();
    assert_eq!(
        error,
        UnlockError::Format(FormatError::UnsupportedFormat(7))
    );
    assert_eq!(error.code(), Some("vault_unsupported_format"));
    assert_eq!(error.to_string(), format7_text);

    // A foreign key source and a later format.
    let foreign = folder(dir.path(), "foreign");
    let header = r#"{"kid":"masterkeyfile:other.key","typ":"JWT","alg":"HS256"}"#;
    std::fs::write(
        format!("{foreign}/vault.cryptomator"),
        token(header, &claims(8)),
    )
    .unwrap();
    let detection = detect_storage(&backend, &foreign).unwrap();
    assert_eq!(detection.key_source, Some("other"));
    assert_eq!(
        detection.problem.as_deref(),
        Some("Nicht unterstützte Schlüsselquelle")
    );
    let nine = folder(dir.path(), "format9");
    let header = r#"{"kid":"masterkeyfile:masterkey.cryptomator","typ":"JWT","alg":"HS256"}"#;
    std::fs::write(
        format!("{nine}/vault.cryptomator"),
        token(header, &claims(9)),
    )
    .unwrap();
    let detection = detect_storage(&backend, &nine).unwrap();
    assert_eq!(
        detection.problem.as_deref(),
        Some("Tresorformat 9 nicht unterstützt")
    );
    // An empty folder is no vault; a vault of se has no problem.
    let empty = folder(dir.path(), "leer");
    let detection = detect_storage(&backend, &empty).unwrap();
    assert!(!detection.is_vault);
    assert_eq!(
        detection.problem.as_deref(),
        Some("Kein Cryptomator-Tresor (vault.cryptomator fehlt)")
    );
    let fixture = site_fixture("Eigener", PASSWORD);
    let detection = detect_storage(&backend, &fixture.root).unwrap();
    assert_eq!((detection.is_vault, detection.problem), (true, None));
    assert_eq!(
        detection.vault_id.as_deref(),
        Some(fixture.record.vault_id.as_str())
    );
}

#[test]
fn vault_task_remove_keeps_files_and_refuses_with_job() {
    let fixture = site_fixture("Archiv", PASSWORD);
    let site = fixture.record.id.clone();
    let general = fixture.dir.path().join("jobs");
    let vault_jobs = fixture.dir.path().join("vault-jobs");
    std::fs::create_dir_all(&general).unwrap();
    std::fs::create_dir_all(&vault_jobs).unwrap();
    std::fs::write(
        general.join("other.conf"),
        "source=/home/x\ntarget=/home/y\n",
    )
    .unwrap();
    let job = format!(
        "kind=vault\nvault_site_a={site}\nvault_site_b=fedcba9876543210fedcba9876543210\n\
         source=vault-store://{site}\ntarget=vault-store://fedcba9876543210fedcba9876543210\n"
    );
    std::fs::write(vault_jobs.join("job-1.conf"), job).unwrap();
    let dirs = [general.clone(), vault_jobs.clone()];
    let secrets = MemorySecrets::default();
    secrets.set(&account(&site), PASSWORD).unwrap();
    let locked = RefCell::new(Vec::new());
    let lock = |id: &str| -> Result<(), String> {
        locked.borrow_mut().push(id.to_string());
        Ok(())
    };

    // A vault sync job uses the site: refused, nothing changed.
    let refused = remove_site(&fixture.store, "Archiv", &dirs, &secrets, &lock).unwrap_err();
    assert_eq!(refused, ManageError::InUse(vec!["job-1".to_string()]));
    assert!(refused.to_string().contains("job-1"));
    assert!(locked.borrow().is_empty());
    assert!(fixture.store.get(&site).unwrap().is_some());
    assert_eq!(secrets.entry(&account(&site)).as_deref(), Some(PASSWORD));

    // A lock that cannot save drive changes keeps the entry.
    std::fs::remove_file(vault_jobs.join("job-1.conf")).unwrap();
    let unsaved = |_: &str| -> Result<(), String> { Err("Nicht gespeicherte Änderungen".into()) };
    let refused = remove_site(&fixture.store, "Archiv", &dirs, &secrets, &unsaved).unwrap_err();
    assert!(matches!(refused, ManageError::Refused(_)));
    assert!(fixture.store.get(&site).unwrap().is_some());

    // Removal: locked first, the password forgotten, the entry gone; the
    // vault stays complete where it is.
    let removed = remove_site(&fixture.store, &site, &dirs, &secrets, &lock).expect("removed");
    assert_eq!(removed.id, site);
    assert_eq!(*locked.borrow(), [site.clone()]);
    assert!(secrets.is_empty());
    assert!(fixture.store.get(&site).unwrap().is_none());
    for name in ["vault.cryptomator", "masterkey.cryptomator", "d"] {
        assert!(Path::new(&fixture.root).join(name).exists(), "{name}");
    }
    let (key, _) = unlock_storage(&LocalBackend::new("/"), &fixture.root, PASSWORD, None)
        .expect("the vault still opens");
    drop(key);
    // Entered again: the same vault, a new site.
    let again = add_site(
        &fixture.store,
        &fixture.root,
        Some("Archiv"),
        &local_resolve,
    )
    .unwrap();
    assert_eq!(again.vault_id, fixture.record.vault_id);
    assert_ne!(again.id, site);
    // A job that names the site by locator counts as well.
    std::fs::write(
        general.join("legacy.conf"),
        format!("source=VAULT://{}/x\n", again.id),
    )
    .unwrap();
    assert_eq!(jobs_using(&again.id, &dirs).unwrap(), ["legacy"]);
}

#[test]
fn vault_task_create_min_password_length_8() {
    let dir = tempfile::tempdir().expect("temporary folder");
    let backend = LocalBackend::new("/");
    let store = RegistryStore::at(
        dir.path().join("vaults"),
        crate::vault::LockSettings::desktop(),
    );
    let root = format!("{}/Kurz", fwd(dir.path()));
    let rule = "Das Passwort braucht mindestens 8 Zeichen";
    // Seven characters: refused before anything is written.
    let error = create_site(
        &store,
        &root,
        Some("Kurz"),
        "1234567",
        220,
        None,
        &local_resolve,
    )
    .unwrap_err();
    assert_eq!(error, ManageError::Invalid(rule.into()));
    assert!(!Path::new(&root).exists());
    assert!(store.load().unwrap().sites.is_empty());
    // Characters, not bytes; decomposed letters count composed (NFC).
    assert_eq!(check_new_password("äöüäöüäö"), Ok(()));
    assert_eq!(
        check_new_password("a\u{308}o\u{308}u\u{308}a\u{308}"),
        Err(rule.into())
    );
    assert_eq!(check_new_password("12345678"), Ok(()));
    // Exactly eight characters create a vault of format 8 with SIV_GCM.
    let record = create_site(
        &store,
        &root,
        Some("Kurz"),
        "12345678",
        220,
        None,
        &local_resolve,
    )
    .expect("created");
    let detection = detect_storage(&backend, &root).unwrap();
    assert_eq!((detection.format, detection.problem), (Some(8), None));
    assert_eq!(
        detection.vault_id.as_deref(),
        Some(record.vault_id.as_str())
    );
    let (_, config) = unlock_storage(&backend, &root, "12345678", None).unwrap();
    assert_eq!(config.combo, crate::cryptomator::CipherCombo::SivGcm);
    assert_eq!(config.shortening_threshold, 220);
    // A password change keeps the rule.
    let short = change_password(&backend, &root, None, "12345678", "kurz", None);
    assert_eq!(short.unwrap_err(), PasswordError::NewPassword(rule.into()));
    // No vault into a folder with content, none beside another vault, no
    // threshold outside 36 to 220.
    let filled = folder(dir.path(), "voll");
    std::fs::write(format!("{filled}/notiz.txt"), "x").unwrap();
    assert!(matches!(
        create_vault(&backend, &filled, "12345678", 220, None),
        Err(ManageError::Refused(_))
    ));
    assert!(matches!(
        create_vault(&backend, &root, "12345678", 220, None),
        Err(ManageError::Refused(_))
    ));
    let fresh = format!("{}/Schwelle", fwd(dir.path()));
    assert!(matches!(
        create_vault(&backend, &fresh, "12345678", 35, None),
        Err(ManageError::Invalid(_))
    ));
    assert!(!Path::new(&fresh).exists());
    // The same place is entered once.
    let twice = create_site(&store, &root, None, "12345678", 220, None, &local_resolve);
    assert!(matches!(twice, Err(ManageError::Refused(_))));
}

#[test]
fn vault_task_recovery_key_reported_unsupported() {
    let info = recovery_key_info();
    assert!(!info.supported);
    assert_eq!(
        info.note,
        "Wiederherstellungsschlüssel von Cryptomator werden in se nicht angezeigt und nicht \
         angenommen, weil die Wortliste nicht frei lizenziert ist. Zum Anzeigen oder Zurücksetzen \
         Cryptomator Desktop verwenden; der Tresor bleibt dafür voll kompatibel."
    );
    assert_eq!(
        CREATE_RECOVERY_HINT,
        "Einen Wiederherstellungsschlüssel erzeugt Cryptomator Desktop für diesen Tresor"
    );
    // No word list and no recovery key code in se (OE1).
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in [
        "vault/os/shared/wordlist.rs",
        "cryptomator/core/recovery_key.rs",
    ] {
        assert!(!source.join(file).exists(), "{file}");
    }
}

#[test]
fn vault_task_remember_password_account_and_forget() {
    let fixture = site_fixture("Privat", PASSWORD);
    let site = fixture.record.id.clone();
    let (store, resolve) = (&fixture.store, &local_resolve);
    assert_eq!(account(&site), format!("smart-explorer:vault:{site}"));
    let register = SessionRegister::new(RegisterOptions {
        publish: false,
        platform: LockPlatform::Desktop,
        background: false,
    });
    let secrets = MemorySecrets::default();
    let flag = || store.get(&site).unwrap().unwrap().remember_password;
    let relock = || register.lock(&site, LockRequest::new(LockReason::Explicit, false));
    let remember = UnlockOptions {
        remember: Some(true),
        available: None,
    };
    // Off by default: an unlock without the choice stores nothing.
    unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        PASSWORD,
        UnlockOptions::default(),
        resolve,
    )
    .unwrap();
    assert!(secrets.is_empty() && !flag());
    assert_eq!(relock(), Ok(true));
    // A wrong password is never stored.
    let wrong = unlock_entered(
        &register,
        store,
        &secrets,
        &site,
        "falsch-falsch",
        remember,
        resolve,
    );
    assert_eq!(
        wrong.unwrap_err(),
        UnlockError::Format(FormatError::WrongPassword)
    );
    assert!(secrets.is_empty() && !flag());
    // A store that refuses: the unlock stands, the flag does not claim it.
    secrets.refuse_set(true);
    let refused = unlock_entered(
        &register, store, &secrets, &site, PASSWORD, remember, resolve,
    )
    .unwrap();
    assert!(refused.remember_error.is_some() && secrets.is_empty() && !flag());
    assert_eq!(relock(), Ok(true));
    secrets.refuse_set(false);
    // Remembered: in the secret store under the account; the vault list
    // keeps only the flag.
    let unlocked = unlock_entered(
        &register, store, &secrets, &site, PASSWORD, remember, resolve,
    )
    .unwrap();
    assert_eq!(unlocked.remember_error, None);
    assert_eq!(secrets.entry(&account(&site)).as_deref(), Some(PASSWORD));
    assert!(flag());
    let entry = std::fs::read_to_string(store.dir().join(format!("{site}.conf"))).unwrap();
    assert!(entry.contains("remember_password=1") && !entry.contains(PASSWORD));
    // `--saved-password` uses it.
    assert_eq!(relock(), Ok(true));
    let saved = unlock_saved(&register, store, &secrets, &site, None, resolve).unwrap();
    assert!(!saved.already);
    // Settings cannot switch it on without a password; „forget“ deletes it.
    let on = SettingsChange {
        remember_password: Some(true),
        ..SettingsChange::default()
    };
    let forget = SettingsChange {
        remember_password: Some(false),
        ..SettingsChange::default()
    };
    update_settings(store, &site, &forget, &secrets).unwrap();
    assert!(secrets.is_empty() && !flag());
    assert!(matches!(
        update_settings(store, &site, &on, &secrets),
        Err(ManageError::Invalid(_))
    ));
    assert_eq!(relock(), Ok(true));
    let none = unlock_saved(&register, store, &secrets, &site, None, resolve).unwrap_err();
    assert_eq!(none, UnlockError::NoSavedPassword);
    assert_eq!(none.code(), Some("vault_no_saved_password"));
    // An unlock with „nicht merken“ forgets a remembered password too.
    unlock_entered(
        &register, store, &secrets, &site, PASSWORD, remember, resolve,
    )
    .unwrap();
    assert!(flag());
    let forget_now = UnlockOptions {
        remember: Some(false),
        available: None,
    };
    unlock_entered(
        &register, store, &secrets, &site, PASSWORD, forget_now, resolve,
    )
    .unwrap();
    assert!(secrets.is_empty() && !flag());
}
