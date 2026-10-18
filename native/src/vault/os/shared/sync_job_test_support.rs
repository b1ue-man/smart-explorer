//! Vaults on disk for the vault sync job tests (stage `vault_sync`): a vault
//! created by se with a password, entered in a vault list of its own, a
//! second side as a seeded copy (Z-a) or as a copy of the folder (Z-b),
//! cleartext views of either side with the vault's key, runs with a local
//! resolver, and the kept vaults of the CryptoFS health check
//! (`SE_VAULT_TASK_KEEP_VAULTS/<test>/<case>/vault`).
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::bisync::RunSettings;
use crate::cryptomator::{Masterkey, VaultConfig};
use crate::syncjobs::SyncJob;
use crate::vault::core::registry_types::{new_site_id, LockSettings, SiteRecord};
use crate::vault::core::VaultSession;
use crate::vfs::{Backend, BackendHandle, LocalBackend};

use super::super::backend::VaultBackend;
use super::super::manage::create_vault;
use super::super::open::ResolveLocation;
use super::super::registry_store::RegistryStore;
use super::super::replica::{create_vault_job, CopyTarget, JobPlace};
use super::super::session_test_support::local_resolve;
use super::super::unlock::session_for;
use super::{run_vault_job_in, VaultRunOutcome, VaultRunRequest};

pub(super) const PASSWORD: &str = "Tresor-Pruefung-2026";

pub(super) fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn local() -> BackendHandle {
    Arc::new(LocalBackend::new("/"))
}

/// A vault created by se, entered as site A of its own vault list.
pub(super) struct Fixture {
    pub(super) dir: tempfile::TempDir,
    pub(super) store: RegistryStore,
    pub(super) site_a: SiteRecord,
    /// Storage root of side A (forward slashes).
    pub(super) root_a: String,
    raw: [u8; 64],
    config: VaultConfig,
    jobs: RefCell<Vec<String>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary folder");
        let store = RegistryStore::at(dir.path().join("vaults"), LockSettings::desktop());
        let root_a = format!("{}/a", fwd(dir.path()));
        let config =
            create_vault(local().as_ref(), &root_a, PASSWORD, 220, None).expect("vault created");
        let id = new_site_id().expect("site ID");
        let site_a = SiteRecord::new(&id, "Tresor A", &root_a, &config.jti, 1, store.defaults());
        store.add(&site_a).expect("site A entered");
        let session = session_for(&id, local().as_ref(), &root_a, PASSWORD).expect("vault unlocks");
        let raw = session
            .with_key(|key| Ok(*key.raw()))
            .expect("key of the session");
        let config = session.config().clone();
        Self {
            dir,
            store,
            site_a,
            root_a,
            raw,
            config,
            jobs: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn path(&self, name: &str) -> String {
        format!("{}/{name}", fwd(self.dir.path()))
    }

    /// The cleartext view of the vault stored at `root` (either side).
    pub(super) fn view(&self, root: &str) -> VaultBackend {
        let session = VaultSession::new(
            &self.site_a.id,
            Masterkey::from_raw(&self.raw),
            self.config.clone(),
        );
        VaultBackend::new(local(), root, session)
    }

    /// Writes a cleartext file (parents created) into the vault at `root`.
    pub(super) fn write(&self, root: &str, path: &str, data: &[u8]) {
        let view = self.view(root);
        if let Some((parent, _)) = path
            .rsplit_once('/')
            .filter(|(parent, _)| !parent.is_empty())
        {
            view.mkdir_all(parent).expect("parent folders");
        }
        let mut writer = view.open_write(path).expect("open_write");
        writer.write_all(data).expect("write");
        writer.flush().expect("flush publishes");
    }

    /// Every cleartext file of the vault at `root` with its bytes; entries
    /// that cannot be listed (an untyped node of an unfinished run) are left
    /// out like in every tolerant listing.
    pub(super) fn tree(&self, root: &str) -> BTreeMap<String, Vec<u8>> {
        fn visit(view: &VaultBackend, dir: &str, out: &mut BTreeMap<String, Vec<u8>>) {
            let listing = crate::vfs::list_dir_tolerant(view, dir).expect("vault listing");
            for entry in listing.entries {
                let path = if dir == "/" {
                    format!("/{}", entry.name)
                } else {
                    format!("{dir}/{}", entry.name)
                };
                if entry.is_dir {
                    visit(view, &path, out);
                } else {
                    let mut bytes = Vec::new();
                    view.open_read(&path)
                        .expect("open_read")
                        .read_to_end(&mut bytes)
                        .expect("read");
                    out.insert(path, bytes);
                }
            }
        }
        let view = self.view(root);
        let mut out = BTreeMap::new();
        visit(&view, "/", &mut out);
        out
    }

    /// Removes a cleartext folder with everything in it.
    pub(super) fn remove_folder(&self, root: &str, path: &str) {
        fn remove(view: &VaultBackend, dir: &str) {
            for entry in view.list_dir(dir).expect("vault listing") {
                let child = format!("{dir}/{}", entry.name);
                if entry.is_dir {
                    remove(view, &child);
                } else {
                    view.remove_file(&child).expect("remove file");
                }
            }
            view.remove_dir(dir).expect("remove folder");
        }
        remove(&self.view(root), path);
    }

    fn unique_id(&self) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let id = format!("vaulttask{}x{nanos:x}", std::process::id());
        self.jobs.borrow_mut().push(id.clone());
        id
    }

    /// Z-a: a vault job from site A to a new seeding copy below the
    /// fixture's replica folder.
    pub(super) fn replica_job(&self) -> (SyncJob, SiteRecord) {
        let replicas = self.dir.path().join("replicas");
        let place = JobPlace {
            replicas_root: &replicas,
            resolve: &local_resolve,
            id_taken: &|_| false,
            save: &|_| Ok(()),
        };
        let created = create_vault_job(
            &self.store,
            &self.site_a.id,
            CopyTarget::Replica,
            None,
            &place,
        )
        .expect("vault job with a copy");
        let mut job = created.job;
        job.id = self.unique_id();
        (job, created.site_b)
    }

    /// Z-b: a copy of A's folder at `name`, entered as a site of the same
    /// vault, and the vault job between A and it.
    pub(super) fn copy_job(&self, name: &str) -> (SyncJob, SiteRecord) {
        let root = self.path(name);
        copy_tree(Path::new(&self.root_a), Path::new(&root));
        let id = new_site_id().expect("site ID");
        let site = SiteRecord::new(&id, name, &root, &self.config.jti, 2, self.store.defaults());
        self.store.add(&site).expect("copy entered");
        let mut job = SyncJob::new_vault("Tresor".into(), &self.site_a.id, &site.id);
        job.id = self.unique_id();
        (job, site)
    }

    pub(super) fn run(&self, job: &SyncJob) -> VaultRunOutcome {
        self.run_with(job, &local_resolve)
    }

    pub(super) fn run_with(&self, job: &SyncJob, resolve: ResolveLocation<'_>) -> VaultRunOutcome {
        let cancel = AtomicBool::new(false);
        run_vault_job_in(
            &self.store,
            resolve,
            VaultRunRequest {
                job,
                dry_run: false,
                cancel: &cancel,
                settings: RunSettings::default(),
                observer: None,
            },
        )
    }

    pub(super) fn record(&self, site: &str) -> SiteRecord {
        self.store.get(site).expect("vault list").expect("site")
    }

    /// Keeps the vault stored at `root` for the CryptoFS health check of
    /// the stage `vault-interop` when the suite asks for it.
    pub(super) fn keep(&self, test: &str, case: &str, root: &str, allowed: &[&str]) {
        let Some(base) = std::env::var_os("SE_VAULT_TASK_KEEP_VAULTS") else {
            return;
        };
        let target = PathBuf::from(base).join(test).join(case);
        let _ = std::fs::remove_dir_all(&target);
        copy_tree(Path::new(root), &target.join("vault"));
        std::fs::write(target.join("password.txt"), PASSWORD).expect("password file");
        if !allowed.is_empty() {
            std::fs::write(target.join("allowed-findings.txt"), allowed.join("\n"))
                .expect("allowed findings");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for id in self.jobs.borrow().iter() {
            let _ = crate::bisync::forget_job_state(id);
            if let Some(log) = crate::bisync::job_log_path(id) {
                let _ = std::fs::remove_file(log);
            }
        }
    }
}

/// Copies a folder tree byte for byte (no links in test vaults).
pub(super) fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("target folder");
    for entry in std::fs::read_dir(from).expect("source folder") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Every stored file below `root` (relative, forward slashes) with its
/// bytes: the encrypted storage as the sync sees it.
pub(super) fn stored(root: &str) -> BTreeMap<String, Vec<u8>> {
    fn visit(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("folder") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                visit(base, &path, out);
            } else {
                let rel = fwd(path.strip_prefix(base).expect("below base"));
                out.insert(rel, std::fs::read(&path).expect("file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    if Path::new(root).is_dir() {
        visit(Path::new(root), Path::new(root), &mut out);
    }
    out
}

/// `stored` without the sync engine's own marker file.
pub(super) fn vault_files(root: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = stored(root);
    files.remove(crate::bisync::REPLICA_MARKER_NAME);
    files
}
