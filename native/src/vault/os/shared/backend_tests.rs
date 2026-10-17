//! Reading, listing, identity and capabilities of the vault backend
//! (VLT-M2, stage `vault_unit`).
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::cryptomator::{encrypt_bytes, file_size, CipherCombo, CHUNK_PAYLOAD};
use crate::vfs::{
    self, Backend, BackendExtensions, BackendHandle, BatchPut, ChangeSignalMode, DeleteDisposition,
    LocalBackend, MtimePrecision, NameLimit, OmissionReason, RootConfinement, Scheme,
    StagedWriteCapabilities, TargetLimits, VersionArchivePolicy, VfsMeta, VfsResult,
    VolumeIdentity,
};

use super::backend_test_support::{pattern, read_with, TestVault};

const CHUNK: usize = CHUNK_PAYLOAD;

fn names(entries: &[VfsMeta]) -> Vec<String> {
    let mut names: Vec<String> = entries.iter().map(|meta| meta.name.clone()).collect();
    names.sort();
    names
}

fn err<T>(result: io::Result<T>) -> io::Error {
    match result {
        Ok(_) => panic!("an error was expected"),
        Err(error) => error,
    }
}

fn roundtrip(combo: CipherCombo) {
    let vault = TestVault::new(combo, 220);
    let backend = &vault.backend;
    backend.create_dir("/Docs").unwrap();
    let sizes = [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK + 17];
    for size in sizes {
        let name = format!("file-{size}.bin");
        let path = format!("/Docs/{name}");
        let data = pattern(size);
        vault.write(&path, &data);
        assert_eq!(vault.read(&path), data, "{combo:?} {size}");
        let meta = backend.stat(&path).unwrap();
        assert_eq!((meta.size, meta.is_dir), (size as u64, false));
        // Header and chunks; an empty file is only its header (C1.7).
        let stored = format!(
            "{}/{}",
            vault.content("/Docs"),
            vault.encrypted("/Docs", &name)
        );
        let stored_len = std::fs::metadata(vault.at(&stored)).unwrap().len();
        assert_eq!(
            stored_len,
            file_size(size as u64, combo),
            "{combo:?} {size}"
        );
    }
    vault.write("/Docs/file-1.bin", b"neu");
    assert_eq!(vault.read("/Docs/file-1.bin"), b"neu");
    let mut expected: Vec<String> = sizes
        .iter()
        .map(|size| format!("file-{size}.bin"))
        .collect();
    expected.sort();
    assert_eq!(names(&backend.list_dir("/Docs").unwrap()), expected);
    // Only encrypted names on the storage, and no stage left behind.
    for stored in vault.stored(&vault.content("/Docs")) {
        assert!(
            stored.ends_with(".c9r") && !stored.contains("file-"),
            "{stored}"
        );
    }
    backend
        .rename("/Docs/file-0.bin", "/Docs/leer.bin")
        .unwrap();
    assert_eq!(vault.read("/Docs/leer.bin"), b"");
    assert_eq!(
        err(backend.stat("/Docs/file-0.bin")).kind(),
        io::ErrorKind::NotFound
    );
    backend.remove_file("/Docs/leer.bin").unwrap();
    assert_eq!(
        err(backend.stat("/Docs/leer.bin")).kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        err(backend.open_read("/Docs")).kind(),
        io::ErrorKind::IsADirectory
    );
    // A new session with the same key reads the same bytes.
    assert_eq!(read_with(&vault.reopened(), "/Docs/file-1.bin"), b"neu");
}

#[test]
fn vault_task_backend_read_write_roundtrip_gcm() {
    roundtrip(CipherCombo::SivGcm);
}

#[test]
fn vault_task_backend_read_write_roundtrip_ctrmac() {
    roundtrip(CipherCombo::SivCtrMac);
}

#[test]
fn vault_task_backend_open_read_at_chunk_boundaries() {
    for combo in [CipherCombo::SivGcm, CipherCombo::SivCtrMac] {
        let vault = TestVault::new(combo, 220);
        let data = pattern(3 * CHUNK + 100);
        vault.write("/big.bin", &data);
        let offsets = [
            0,
            1,
            100,
            CHUNK - 1,
            CHUNK,
            CHUNK + 1,
            2 * CHUNK,
            3 * CHUNK,
            3 * CHUNK + 99,
            3 * CHUNK + 100,
            3 * CHUNK + 500,
            10 * CHUNK,
        ];
        for offset in offsets {
            let mut reader = vault
                .backend
                .open_read_at("/big.bin", None, offset as u64)
                .unwrap()
                .expect("the local storage starts mid-file");
            let mut out = Vec::new();
            reader.read_to_end(&mut out).unwrap();
            let expected = data.get(offset..).unwrap_or(&[]);
            assert_eq!(out, expected, "{combo:?} offset {offset}");
        }
    }
}

#[test]
fn vault_task_backend_symlink_listed_never_followed() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    vault.write("/ziel.txt", b"inhalt");
    // A link as CryptoFS stores it: `<name>.c9r/symlink.c9r`, target encrypted.
    let node = vault.at(&format!(
        "{}/{}",
        vault.content("/"),
        vault.encrypted("/", "link")
    ));
    std::fs::create_dir(&node).unwrap();
    let target = encrypt_bytes(&vault.session.cryptor(), b"ziel.txt").unwrap();
    std::fs::write(format!("{node}/symlink.c9r"), &target).unwrap();
    let listing = backend.list_dir("/").unwrap();
    let link = listing.iter().find(|meta| meta.name == "link").unwrap();
    assert!(link.is_symlink && !link.is_dir);
    assert_eq!(link.size, 8);
    assert!(backend.stat("/link").unwrap().is_symlink);
    assert_eq!(
        err(backend.open_read("/link")).kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        err(backend.list_dir("/link")).kind(),
        io::ErrorKind::NotADirectory
    );
    assert_eq!(
        err(backend.stat("/link/x")).kind(),
        io::ErrorKind::NotADirectory
    );
    assert!(backend.open_write("/link").is_err());
    // Removing the link removes only the link.
    backend.remove_file("/link").unwrap();
    assert!(!Path::new(&node).exists());
    assert_eq!(vault.read("/ziel.txt"), b"inhalt");
}

#[test]
fn vault_task_backend_foreign_conflict_copy_alias_not_renamed() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    let content = vault.content("/");
    vault.write("/report.txt", b"Fassung A");
    let name = vault.encrypted("/", "report.txt");
    let conflict = format!(
        "{} (Conflict 2026-10-17).c9r",
        name.strip_suffix(".c9r").unwrap()
    );
    std::fs::copy(
        vault.at(&format!("{content}/{name}")),
        vault.at(&format!("{content}/{conflict}")),
    )
    .unwrap();
    vault.write("/report.txt", b"Fassung B");
    // Listed under CryptoFS's display name; nothing is renamed.
    let listed = names(&backend.list_dir("/").unwrap());
    assert_eq!(listed, ["report (1).txt", "report.txt"]);
    assert_eq!(vault.read("/report (1).txt"), b"Fassung A");
    assert_eq!(vault.read("/report.txt"), b"Fassung B");
    assert!(vault.stored(&content).contains(&conflict));
    // Alone, the conflict copy shows its plain name, still not renamed.
    backend.remove_file("/report.txt").unwrap();
    assert_eq!(names(&backend.list_dir("/").unwrap()), ["report.txt"]);
    assert_eq!(vault.read("/report.txt"), b"Fassung A");
    assert_eq!(vault.stored(&content), [conflict.clone()]);
    // A folder copy shares the directory ID: listed, readable, never moved.
    backend.create_dir("/Docs").unwrap();
    vault.write("/Docs/a.txt", b"a");
    let docs = vault.encrypted("/", "Docs");
    let docs_copy = format!("{} (1).c9r", docs.strip_suffix(".c9r").unwrap());
    std::fs::create_dir(vault.at(&format!("{content}/{docs_copy}"))).unwrap();
    std::fs::copy(
        vault.at(&format!("{content}/{docs}/dir.c9r")),
        vault.at(&format!("{content}/{docs_copy}/dir.c9r")),
    )
    .unwrap();
    let listed = names(&backend.list_dir("/").unwrap());
    assert_eq!(listed, ["Docs", "Docs (1)", "report.txt"]);
    assert_eq!(vault.read("/Docs (1)/a.txt"), b"a");
    let refused = err(backend.rename("/Docs (1)", "/Andere"));
    assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        err(backend.remove_dir("/Docs (1)")).kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(vault.stored(&content).contains(&docs_copy));
    let tolerant = vfs::list_dir_tolerant(backend, "/").unwrap();
    assert!(tolerant.omitted.is_empty(), "{:?}", tolerant.omitted);
}

#[test]
fn vault_task_backend_undecryptable_reported_incomplete() {
    let vault = TestVault::new(CipherCombo::SivGcm, 220);
    let backend = &vault.backend;
    vault.write("/gut.txt", b"ok");
    let content = vault.content("/");
    // Base64 of the right shape that is no name of this vault.
    let forged = format!("{}.c9r", "A".repeat(40));
    std::fs::write(vault.at(&format!("{content}/{forged}")), b"x").unwrap();
    // Not part of the vault: invisible as in CryptoFS.
    std::fs::write(vault.at(&format!("{content}/desktop.ini")), b"").unwrap();
    std::fs::write(vault.at(&format!("{content}/._{forged}")), b"").unwrap();
    std::fs::write(vault.at(&format!("{content}/x.c9u")), b"").unwrap();
    let listing = vfs::list_dir_tolerant(backend, "/").unwrap();
    assert_eq!(names(&listing.entries), ["gut.txt"]);
    assert_eq!(listing.omitted.len(), 1, "{:?}", listing.omitted);
    assert_eq!(listing.omitted[0].rel, forged);
    assert_eq!(listing.omitted[0].reason, OmissionReason::Unreadable);
    // The strict listing stops instead of returning less (tree operations).
    let strict = err(backend.list_dir("/"));
    assert_eq!(strict.kind(), io::ErrorKind::InvalidData);
    assert!(!strict.to_string().contains("gut"), "{strict}");
}

/// A storage that claims everything a backend can claim.
struct Generous(LocalBackend);

impl Backend for Generous {
    fn scheme(&self) -> Scheme {
        Scheme::Local
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
    fn has_duplicate_file_names(&self) -> bool {
        true
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
    fn delete_disposition(&self) -> DeleteDisposition {
        DeleteDisposition::Recycle
    }
    fn parallelism(&self) -> usize {
        3
    }
    fn transfer_ceiling(&self, _path: &str) -> Option<usize> {
        Some(2)
    }
    fn concurrent_read_write(&self) -> bool {
        false
    }
    fn rename_overwrites(&self) -> bool {
        true
    }
    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities {
            create: true,
            replace: false,
            namespace_replace: true,
        }
    }
    fn case_sensitive_paths(&self, _root: &str) -> bool {
        false
    }
    fn root_confinement(&self, _root: &str) -> RootConfinement {
        RootConfinement::Enforced
    }
    fn is_local(&self) -> bool {
        true
    }
    fn provides_content_hash(&self) -> bool {
        true
    }
    fn supports_changes(&self) -> bool {
        true
    }
    fn supports_walk_tree(&self) -> bool {
        true
    }
    fn supports_bulk_tree(&self) -> bool {
        true
    }
    fn supports_search(&self) -> bool {
        true
    }
    fn supports_walk_hashed(&self) -> bool {
        true
    }
}

impl BackendExtensions for Generous {
    fn version_archive_policy(&self) -> VersionArchivePolicy {
        VersionArchivePolicy::AppPrivate
    }
    fn target_limits(&self, _root: &str) -> TargetLimits {
        TargetLimits {
            windows_names: true,
            max_name: Some(NameLimit::Bytes(255)),
            max_file_size: Some(u64::from(u32::MAX)),
            mtime_precision: MtimePrecision::TwoSeconds,
        }
    }
    fn unix_mode(&self, _path: &str) -> VfsResult<Option<u32>> {
        Ok(Some(0o644))
    }
    fn volume_identity(&self, _root: &str) -> VfsResult<Option<VolumeIdentity>> {
        Ok(Some(VolumeIdentity {
            volume_id: "abc".into(),
            relative_path: String::new(),
            fs_type: "ext4".into(),
        }))
    }
    fn supports_duplicate_search(&self, _root: &str) -> VfsResult<bool> {
        Ok(true)
    }
    fn supports_hash_walk(&self, _root: &str) -> VfsResult<bool> {
        Ok(true)
    }
    fn supports_recycle(&self, _path: &str) -> VfsResult<bool> {
        Ok(true)
    }
    fn supports_remove_tree(&self, _path: &str) -> VfsResult<bool> {
        Ok(true)
    }
    fn change_signal_mode(&self, _root: &str) -> VfsResult<Option<ChangeSignalMode>> {
        Ok(Some(ChangeSignalMode::Push))
    }
}

#[test]
fn vault_task_backend_capabilities_never_exceed_inner() {
    let vault = TestVault::on(CipherCombo::SivGcm, 220, |_| -> BackendHandle {
        Arc::new(Generous(LocalBackend::new("/")))
    });
    let backend = &vault.backend;
    let inner = Generous(LocalBackend::new("/"));
    let root = vault.root.as_str();
    // Passed through (C2.5).
    assert_eq!(
        backend.staged_write_capabilities("/"),
        inner.staged_write_capabilities(root)
    );
    assert_eq!(
        backend.probe_staged_write_capabilities("/").unwrap(),
        inner.staged_write_capabilities(root)
    );
    let mount = backend.mount_path_capabilities("/").unwrap();
    assert_eq!(mount.staged_write, inner.staged_write_capabilities(root));
    assert_eq!(mount.root_confinement, RootConfinement::Enforced);
    assert_eq!(backend.root_confinement("/"), RootConfinement::Enforced);
    assert!(!backend.concurrent_read_write());
    assert_eq!(backend.transfer_ceiling("/x"), Some(2));
    assert_eq!(backend.parallelism(), 3);
    assert!(backend.has_duplicate_file_names());
    let (storage, storage_root) = backend.storage();
    assert_eq!(backend.flow_key("/x"), storage.flow_key(storage_root));
    // Fixed by the vault, never more than the storage.
    assert!(!backend.rename_overwrites());
    assert!(backend.case_sensitive_paths("/"));
    assert!(!backend.is_local());
    assert!(!backend.provides_content_hash());
    assert!(!backend.supports_changes());
    assert!(!backend.supports_search());
    assert!(!backend.supports_walk_tree());
    assert!(!backend.supports_bulk_tree());
    assert!(!backend.supports_walk_hashed());
    assert_eq!(backend.delete_disposition(), DeleteDisposition::Permanent);
    let batch = backend.put_batch(
        &[BatchPut {
            path: "/a".into(),
            size: 0,
        }],
        &mut io::empty(),
    );
    assert_eq!(err(batch).kind(), io::ErrorKind::Unsupported);
    // Extensions: the vault's own, never the storage's.
    assert_eq!(
        vfs::version_archive_policy(backend),
        VersionArchivePolicy::Provider
    );
    assert!(vfs::volume_identity(backend, "/").unwrap().is_none());
    assert!(!vfs::supports_recycle(backend, "/").unwrap());
    assert!(!vfs::supports_remove_tree(backend, "/").unwrap());
    assert!(!vfs::supports_hash_walk(backend, "/").unwrap());
    assert!(!vfs::supports_duplicate_search(backend, "/").unwrap());
    assert_eq!(vfs::change_signal_mode(backend, "/").unwrap(), None);
    assert_eq!(vfs::unix_mode(backend, "/").unwrap(), None);
    let limits = vfs::target_limits(backend, "/");
    assert!(!limits.windows_names);
    assert_eq!(limits.max_name, None);
    assert_eq!(limits.mtime_precision, MtimePrecision::TwoSeconds);
    let max = limits.max_file_size.unwrap();
    assert!(max < u64::from(u32::MAX));
    assert!(file_size(max, CipherCombo::SivGcm) <= u64::from(u32::MAX));
    assert!(file_size(max + 1, CipherCombo::SivGcm) > u64::from(u32::MAX));
}

#[test]
fn vault_task_backend_identity_distinct_from_inner_and_other_vaults() {
    let first = TestVault::new(CipherCombo::SivGcm, 220);
    let second = TestVault::new(CipherCombo::SivGcm, 220);
    let inner = LocalBackend::new("/");
    let identity = first.backend.state_identity();
    assert_eq!(
        identity,
        format!("vault:{}#{}", inner.state_identity(), first.root)
    );
    assert_ne!(identity, inner.state_identity());
    assert_ne!(
        first.backend.namespace_identity(),
        inner.namespace_identity()
    );
    assert_ne!(identity, second.backend.state_identity());
    assert_ne!(
        first.backend.namespace_identity(),
        second.backend.namespace_identity()
    );
    // The same vault through another session is the same place.
    assert_eq!(first.reopened().state_identity(), identity);
    assert_eq!(first.backend.scheme(), Scheme::Vault);
    assert_eq!(first.backend.root_display(), "/");
}
