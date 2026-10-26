//! B1: retained mount access consults the current isolated registry record.
use super::*;
use crate::vault::{LockSettings, OpenPolicy, SiteRecord};

struct AccessFixture {
    root: tempfile::TempDir,
    record: SiteRecord,
    binding: RootBinding,
}
impl AccessFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let record = SiteRecord::new("0123456789abcdef0123456789abcdef", "Fixture",
            "sftp://fixture/Vault", "fixture-vault", 0, LockSettings::desktop());
        let binding = RootBinding {
            site: record.id.clone(), vault: record.vault_id.clone(), location: record.location.clone(),
            backend: "Sftp".into(), namespace: "fixture-server".into(), account: "fixture-account".into(),
            root: "/Vault".into(), object: Some("fixture-object".into()),
        };
        Self { root, record, binding }
    }
    fn persist(&mut self, policy: OpenPolicy) {
        store_policy(&mut self.record, &policy).unwrap();
        let store = RegistryStore::at(self.root.path().to_path_buf(), LockSettings::desktop());
        if store.get(&self.record.id).unwrap().is_none() {
            store.add(&self.record).unwrap();
        } else {
            store.update(&self.record.id, |record| { *record = self.record.clone(); Ok(()) }).unwrap();
        }
    }
    fn access(&self) -> MountAccess {
        MountAccess { binding: self.binding.clone(), dir: self.root.path().to_path_buf(), trusted: true }
    }
}

#[test]
fn batches_task_vault_retained_access_observes_root_revocation_with_cleartext_consent_retained() {
    let mut fixture = AccessFixture::new();
    fixture.persist(OpenPolicy { trusted_root: Some(fixture.binding.clone()), cleartext_copy: Some(fixture.binding.clone()) });
    let access = fixture.access();
    access.check().unwrap();
    fixture.persist(OpenPolicy { trusted_root: None, cleartext_copy: Some(fixture.binding.clone()) });
    let error = access.check().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains(ROOT_TRUST_REQUIRED));
    fixture.persist(OpenPolicy { trusted_root: Some(fixture.binding.clone()), cleartext_copy: None });
    access.check().unwrap();
}

#[test]
fn batches_task_vault_retained_access_rejects_rebound_policy_and_changed_site_record() {
    let mut fixture = AccessFixture::new();
    let access = fixture.access();
    for field in ["site", "vault", "location", "backend", "namespace", "account", "root", "object"] {
        let mut changed = serde_json::to_value(&fixture.binding).unwrap();
        changed[field] = serde_json::json!("replacement");
        fixture.persist(OpenPolicy { trusted_root: Some(serde_json::from_value(changed).unwrap()), cleartext_copy: None });
        assert_eq!(access.check().unwrap_err().kind(), io::ErrorKind::PermissionDenied, "{field}");
    }
    fixture.persist(OpenPolicy { trusted_root: Some(fixture.binding.clone()), cleartext_copy: None });
    access.check().unwrap();
    fixture.record.location = "sftp://replacement/Vault".into();
    fixture.persist(OpenPolicy { trusted_root: Some(fixture.binding.clone()), cleartext_copy: None });
    assert_eq!(access.check().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    std::fs::remove_file(fixture.root.path().join(format!("{}.conf", fixture.record.id))).unwrap();
    assert_eq!(access.check().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}

/// Real LocalBackend metadata I/O with provider identity/capability facts selected
/// by the fixture. No network service, provider credentials or policy reimplementation.
struct IdentityBackend {
    local: crate::vfs::LocalBackend,
    account: String,
    namespace: String,
    object: String,
    capabilities: crate::vfs::MountPathCapabilities,
}
impl IdentityBackend {
    fn new() -> Self {
        Self {
            local: crate::vfs::LocalBackend::new("/"), account: "account-a".into(),
            namespace: "namespace-a".into(), object: "object-a".into(),
            capabilities: crate::vfs::MountPathCapabilities {
                staged_write: crate::vfs::StagedWriteCapabilities::complete(),
                root_confinement: crate::vfs::RootConfinement::Unverified,
            },
        }
    }
}
impl Backend for IdentityBackend {
    fn scheme(&self) -> crate::vfs::Scheme { crate::vfs::Scheme::Sftp }
    fn root_display(&self) -> String { "/".into() }
    fn state_identity(&self) -> String { self.account.clone() }
    fn namespace_identity(&self) -> String { self.namespace.clone() }
    fn item_id(&self, _: &str) -> io::Result<Option<String>> { Ok(Some(self.object.clone())) }
    fn mount_path_capabilities(&self, _: &str) -> io::Result<crate::vfs::MountPathCapabilities> { Ok(self.capabilities) }
    fn delete_disposition(&self) -> DeleteDisposition { self.local.delete_disposition() }
    fn list_dir(&self, path: &str) -> io::Result<Vec<crate::vfs::VfsMeta>> { self.local.list_dir(path) }
    fn stat(&self, path: &str) -> io::Result<crate::vfs::VfsMeta> { self.local.stat(path) }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn std::io::Read + Send>> { self.local.open_read(path) }
    fn open_write(&self, path: &str) -> io::Result<Box<dyn std::io::Write + Send>> { self.local.open_write(path) }
    fn rename(&self, from: &str, to: &str) -> io::Result<()> { self.local.rename(from, to) }
    fn remove_file(&self, path: &str) -> io::Result<()> { self.local.remove_file(path) }
    fn remove_dir(&self, path: &str) -> io::Result<()> { self.local.remove_dir(path) }
    fn mkdir_all(&self, path: &str) -> io::Result<()> { self.local.mkdir_all(path) }
}

fn detected_vault(root: &std::path::Path, vault: &str) -> String {
    use base64::Engine;
    let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    std::fs::create_dir_all(root).unwrap();
    let header = r#"{"kid":"masterkeyfile:masterkey.cryptomator","typ":"JWT","alg":"HS256"}"#;
    let claims = serde_json::json!({"jti":vault,"format":8,"cipherCombo":"SIV_GCM","shorteningThreshold":220});
    // Detection intentionally does not unlock or authenticate a JWT signature.
    let token = format!("{}.{}.{}", encoder.encode(header), encoder.encode(claims.to_string()), encoder.encode("fixture-signature"));
    std::fs::write(root.join("vault.cryptomator"), token).unwrap();
    root.to_string_lossy().replace('\\', "/")
}

#[test]
fn batches_task_vault_resolved_policy_requires_exact_provider_account_root_and_object() {
    let directory = tempfile::tempdir().unwrap();
    let root = detected_vault(&directory.path().join("first"), "resolved-vault");
    let second_root = detected_vault(&directory.path().join("second"), "resolved-vault");
    let mut backend = IdentityBackend::new();
    let mut record = SiteRecord::new("0123456789abcdef0123456789abcdef", "Resolved fixture",
        "sftp://fixture/vault", "resolved-vault", 0, LockSettings::desktop());
    let initial = policy_for(&record, &backend, &root).unwrap();
    assert!(!initial.allowed());
    assert!(!initial.cleartext_copy);
    assert_eq!(initial.binding.backend, "Sftp");
    assert_eq!(initial.binding.account, "account-a");
    assert_eq!(initial.binding.namespace, "namespace-a");
    assert_eq!(initial.binding.root, root);
    assert_eq!(initial.binding.object.as_deref(), Some("object-a"));
    store_policy(&mut record, &OpenPolicy { trusted_root: Some(initial.binding.clone()), cleartext_copy: None }).unwrap();
    let trusted = policy_for(&record, &backend, &root).unwrap();
    assert!(trusted.allowed());
    assert!(!trusted.cleartext_copy);
    store_policy(&mut record, &OpenPolicy { trusted_root: Some(initial.binding.clone()), cleartext_copy: Some(initial.binding) }).unwrap();
    assert!(policy_for(&record, &backend, &root).unwrap().cleartext_copy);
    for field in ["account", "namespace", "object", "root"] {
        backend = IdentityBackend::new();
        match field {
            "account" => backend.account = "account-b".into(),
            "namespace" => backend.namespace = "namespace-b".into(),
            "object" => backend.object = "object-b".into(),
            _ => {},
        }
        let observed = policy_for(&record, &backend, if field == "root" { &second_root } else { &root }).unwrap();
        assert!(!observed.allowed(), "{field}");
        assert!(!observed.cleartext_copy, "{field}");
    }
    backend = IdentityBackend::new();
    backend.capabilities.root_confinement = crate::vfs::RootConfinement::Enforced;
    store_policy(&mut record, &OpenPolicy::default()).unwrap();
    let enforced = policy_for(&record, &backend, &root).unwrap();
    assert!(enforced.allowed());
    assert!(!enforced.trusted);
    assert!(!enforced.cleartext_copy);
    backend.capabilities.staged_write = crate::vfs::StagedWriteCapabilities::default();
    assert!(policy_for(&record, &backend, &root).unwrap().read_only);
    record.vault_id = "different-vault".into();
    assert!(policy_for(&record, &backend, &root).is_err());
}

#[test]
fn batches_task_vault_stale_settings_save_cannot_restore_revoked_root_consent() {
    let mut fixture = AccessFixture::new();
    fixture.persist(OpenPolicy { trusted_root: Some(fixture.binding.clone()), cleartext_copy: Some(fixture.binding.clone()) });
    let stale = fixture.record.clone();
    let access = fixture.access();
    access.check().unwrap();
    let store = RegistryStore::at(fixture.root.path().to_path_buf(), LockSettings::desktop());
    store.update(&stale.id, |record| {
        let mut policy = stored_policy(record)?;
        policy.trusted_root = None;
        store_policy(record, &policy)
    }).unwrap();
    store.save(&stale).unwrap();
    let current = store.get(&stale.id).unwrap().unwrap();
    let policy = stored_policy(&current).unwrap();
    assert!(policy.trusted_root.is_none());
    assert_eq!(policy.cleartext_copy, Some(fixture.binding));
    assert_eq!(access.check().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}
