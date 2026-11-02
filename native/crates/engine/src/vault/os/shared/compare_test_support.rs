//! Vaults on disk for the tests of the comparison mode and „Tresor prüfen“
//! (stage `vault_compare`): a vault created by se with a password, a second
//! side as a copy of its folder (the same vault under another site), the
//! vault job between them (saved in the vault job folder, as the engine
//! reads its options from there) after one run that records the baseline,
//! the session of the vault in the process table, cleartext views of either
//! side, and helpers that read the encrypted storage as the sync sees it.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::bisync::RunSettings;
use crate::cryptomator::{Masterkey, VaultConfig};
use crate::syncjobs::SyncJob;
use crate::vault::core::compare_model::{CompareGroup, CompareItem, CompareView};
use crate::vault::core::registry_types::{new_site_id, LockSettings, SiteRecord};
use crate::vault::core::VaultSession;
use crate::vfs::{Backend, BackendHandle, LocalBackend};

use super::backend::VaultBackend;
use super::compare::compare_job_in;
use super::compare_actions::{resolve_item_in, ActionEnv, ResolveReport, ResolveRequest};
use super::manage::create_vault;
use super::open::{publish_session, withdraw_session};
use super::registry_store::RegistryStore;
use super::session_test_support::local_resolve;
use super::sync_job::{run_vault_job_in, VaultRunOutcome, VaultRunRequest};
use super::unlock::session_for;

pub(super) const PASSWORD: &str = "Tresor-Vergleich-2026";

pub(super) fn fwd(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn local() -> BackendHandle {
    Arc::new(LocalBackend::new("/"))
}

pub(super) fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

/// Two sides of one vault with a job between them.
pub(super) struct Fx {
    pub(super) dir: tempfile::TempDir,
    pub(super) store: RegistryStore,
    pub(super) site_a: SiteRecord,
    pub(super) site_b: SiteRecord,
    pub(super) root_a: String,
    pub(super) root_b: String,
    pub(super) job: SyncJob,
    pub(super) session: Arc<VaultSession>,
    raw: [u8; 64],
    config: VaultConfig,
}

impl Fx {
    /// The vault with four files, its copy, the job and a first run.
    pub(super) fn new() -> Self {
        let fx = Self::bare();
        fx.write(&fx.root_a, "/Bericht.txt", b"Inhalt A");
        fx.write(&fx.root_a, "/Ordner/Tief/x.bin", &pattern(70_000));
        fx.write(&fx.root_a, "/leer.txt", b"");
        fx.write(&fx.root_a, "/konflikt.txt", b"Basis");
        fx.settle()
    }

    /// The vault without files: the copy and the job exist, no run yet.
    pub(super) fn bare() -> Self {
        let dir = tempfile::tempdir().expect("temporary folder");
        let store = RegistryStore::at(dir.path().join("vaults"), LockSettings::desktop());
        let root_a = format!("{}/a", fwd(dir.path()));
        let config =
            create_vault(local().as_ref(), &root_a, PASSWORD, 220, None).expect("vault created");
        let id_a = new_site_id().expect("site ID");
        let site_a = SiteRecord::new(&id_a, "Tresor A", &root_a, &config.jti, 1, store.defaults());
        store.add(&site_a).expect("site A entered");
        let session =
            session_for(&id_a, local().as_ref(), &root_a, PASSWORD).expect("vault unlocks");
        let raw = session
            .with_key(|key| Ok(*key.raw()))
            .expect("key of the session");
        let config = session.config().clone();
        publish_session(Arc::clone(&session));
        // The second side is made by `settle` (after the files exist).
        let id_b = new_site_id().expect("site ID");
        let root_b = format!("{}/b", fwd(dir.path()));
        let site_b = SiteRecord::new(&id_b, "Tresor B", &root_b, &config.jti, 2, store.defaults());
        let mut job = SyncJob::new_vault("Tresor".into(), &id_a, &id_b);
        job.id = unique_id();
        Self {
            dir,
            store,
            site_a,
            site_b,
            root_a,
            root_b,
            job,
            session,
            raw,
            config,
        }
    }

    /// Copies side A to side B, enters B, saves the job and runs it once, so
    /// both sides and the baseline agree.
    pub(super) fn settle(self) -> Self {
        copy_tree(Path::new(&self.root_a), Path::new(&self.root_b));
        self.store.add(&self.site_b).expect("site B entered");
        crate::syncjobs::upsert(&self.job).expect("vault job saved");
        let first = self.run();
        assert!(first.unavailable.is_none(), "{:?}", first.unavailable);
        assert!(
            first.outcome.errors.is_empty(),
            "{:?}",
            first.outcome.errors
        );
        assert!(first.outcome.conflicts.is_empty());
        self
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

    pub(super) fn run(&self) -> VaultRunOutcome {
        let cancel = AtomicBool::new(false);
        run_vault_job_in(
            &self.store,
            &local_resolve,
            VaultRunRequest {
                job: &self.job,
                dry_run: false,
                cancel: &cancel,
                settings: RunSettings::default(),
                observer: None,
            },
        )
    }

    /// The comparison of the job with the groups asked for.
    pub(super) fn compare(&self, groups: &[CompareGroup]) -> CompareView {
        compare_job_in(
            &self.store,
            &local_resolve,
            &self.job,
            groups,
            &AtomicBool::new(false),
        )
        .expect("comparison")
    }

    /// An action on an entry, as `Resolve` asks for it.
    pub(super) fn resolve(
        &self,
        item: &CompareItem,
        action: &str,
        confirmed: bool,
    ) -> io::Result<ResolveReport> {
        self.resolve_request(Some(&item.id), action, false, confirmed)
    }

    /// Keeps the vault stored at `root` for the CryptoFS health check of the
    /// stage `vault_interop` when the suite asks for it
    /// (`SE_VAULT_TASK_KEEP_VAULTS/<test>/<case>/vault`, `password.txt`).
    pub(super) fn keep(&self, test: &str, case: &str, root: &str) {
        let Some(base) = std::env::var_os("SE_VAULT_TASK_KEEP_VAULTS") else {
            return;
        };
        let target = Path::new(&base).join(test).join(case);
        let _ = std::fs::remove_dir_all(&target);
        copy_tree(Path::new(root), &target.join("vault"));
        std::fs::write(target.join("password.txt"), PASSWORD).expect("password file");
    }

    /// An action on an entry of the check of side A's site.
    pub(super) fn resolve_check(
        &self,
        item: &str,
        action: &str,
        confirmed: bool,
    ) -> io::Result<ResolveReport> {
        resolve_item_in(
            &ActionEnv {
                store: &self.store,
                resolve: &local_resolve,
            },
            ResolveRequest {
                job: &format!("check:{}", self.site_a.id),
                item: Some(item),
                action,
                all: false,
                confirmed,
            },
            &AtomicBool::new(false),
        )
    }

    pub(super) fn resolve_request(
        &self,
        item: Option<&str>,
        action: &str,
        all: bool,
        confirmed: bool,
    ) -> io::Result<ResolveReport> {
        resolve_item_in(
            &ActionEnv {
                store: &self.store,
                resolve: &local_resolve,
            },
            ResolveRequest {
                job: &self.job.id,
                item,
                action,
                all,
                confirmed,
            },
            &AtomicBool::new(false),
        )
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        withdraw_session(&self.site_a.id);
        withdraw_session(&self.site_b.id);
        let _ = crate::syncjobs::remove(&self.job.id);
        let _ = crate::bisync::forget_job_state(&self.job.id);
        if let Some(log) = crate::bisync::job_log_path(&self.job.id) {
            let _ = std::fs::remove_file(log);
        }
    }
}

/// The entry of `path` in a comparison.
pub(super) fn item<'a>(view: &'a CompareView, path: &str) -> &'a CompareItem {
    view.items
        .iter()
        .find(|item| item.path == path)
        .unwrap_or_else(|| {
            let known: Vec<(&str, &str)> = view
                .items
                .iter()
                .map(|item| (item.path.as_str(), item.group.key()))
                .collect();
            panic!("no entry {path}: {known:?}")
        })
}

fn unique_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("vaulttask{}x{nanos:x}", std::process::id())
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
    out.remove(crate::bisync::REPLICA_MARKER_NAME);
    out
}

/// A cleartext file is replaced by another version with an older or newer
/// time: the sync compares size and time, so a pause makes the time differ.
pub(super) fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(30));
}
