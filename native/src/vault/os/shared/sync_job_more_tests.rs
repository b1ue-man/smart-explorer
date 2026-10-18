//! Vault sync jobs, second part (stage `vault_sync`): unreachable sides,
//! failed phase-1 actions, the full scan of every run (R4a), runs without a
//! key, server hashes and the job log.
use std::io::{Read, Write};
use std::sync::Mutex;

use super::test_support::{stored, vault_files, Fixture};
use super::*;
use crate::bisync::test_remote::{FakeRemote, REMOTE_ROOT};
use crate::vfs::{LocalBackend, Scheme, VfsMeta, VfsResult};

fn local_resolve(location: &str) -> Result<(BackendHandle, String), String> {
    Ok((Arc::new(LocalBackend::new("/")), location.to_string()))
}

#[test]
fn vault_task_sync_unreachable_side_no_deletes() {
    let f = Fixture::new();
    f.write(&f.root_a, "/eins.txt", b"1");
    f.write(&f.root_a, "/Ordner/zwei.txt", b"2");
    let (job, site_b) = f.replica_job();
    assert!(f.run(&job).unavailable.is_none());
    let root_b = site_b.location.clone();
    let before_a = vault_files(&f.root_a);

    // The storage of B cannot be reached.
    let refuse_b = |location: &str| {
        if location == root_b {
            Err("Verbindung abgelehnt".to_string())
        } else {
            local_resolve(location)
        }
    };
    let outcome = f.run_with(&job, &refuse_b);
    assert!(outcome.unavailable.is_some());
    assert_eq!(outcome.planned_with_rules, 0);
    assert_eq!(vault_files(&f.root_a), before_a);

    // B was emptied (a vanished copy that is no longer seeding).
    for entry in std::fs::read_dir(&root_b).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            std::fs::remove_dir_all(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
    let outcome = f.run(&job);
    let reason = outcome.unavailable.expect("an empty side is not available");
    assert!(reason.contains("vault.cryptomator"), "{reason}");
    assert_eq!(vault_files(&f.root_a), before_a, "no deletion on A");

    // B's folder is gone completely.
    std::fs::remove_dir_all(&root_b).unwrap();
    assert!(f.run(&job).unavailable.is_some());
    assert_eq!(vault_files(&f.root_a), before_a);
    assert_eq!(f.tree(&f.root_a).len(), 2);
}

#[test]
fn vault_task_sync_phase1_failure_keeps_phase2_planned() {
    let f = Fixture::new();
    f.write(&f.root_a, "/alt.txt", b"alt");
    let (job, site_b) = f.replica_job();
    assert!(f.run(&job).unavailable.is_none());
    let root_b = site_b.location.clone();
    // A new folder on A: content folder with `dirid.c9r` (phase 1), node in
    // the root's content folder (phase 2).
    f.write(&f.root_a, "/Neu/datei.txt", b"im neuen Ordner");
    // B's storage refuses to write any `dirid.c9r`.
    let failing_b = |location: &str| -> Result<(BackendHandle, String), String> {
        if location == root_b {
            let remote = FakeRemote::new(std::path::Path::new(&root_b), "vault-b")
                .failing_writes_to("dirid");
            Ok((Arc::new(remote), REMOTE_ROOT.to_string()))
        } else {
            local_resolve(location)
        }
    };
    let failed = f.run_with(&job, &failing_b);
    assert!(failed.unavailable.is_none());
    assert!(failed.outcome.stats.errors > 0, "the dirid copy failed");
    let held: Vec<&str> = failed
        .outcome
        .deferred
        .iter()
        .filter(|(_, reason)| reason == crate::bisync::plan_rules::HELD_BACK)
        .map(|(rel, _)| rel.as_str())
        .collect();
    assert_eq!(held.len(), 1, "{:?}", failed.outcome.deferred);
    assert!(held[0].ends_with("/dir.c9r"), "the node waits: {held:?}");
    assert!(
        !vault_files(&root_b).contains_key(held[0]),
        "no node without its content"
    );
    assert!(!f.tree(&root_b).contains_key("/Neu/datei.txt"));
    // The node stays planned: the next run with a working B completes it.
    let next = f.run(&job);
    assert!(next.unavailable.is_none());
    assert!(next.outcome.errors.is_empty(), "{:?}", next.outcome.errors);
    assert_eq!(f.tree(&root_b), f.tree(&f.root_a));
    assert_eq!(f.tree(&root_b)["/Neu/datei.txt"], b"im neuen Ordner");
    f.keep(
        "vault_task_sync_phase1_failure_keeps_phase2_planned",
        "b",
        &root_b,
        &[],
    );
}

#[test]
fn vault_task_sync_never_incremental() {
    // Engine level (R4a in `incremental.rs`): a one-way mirror with rules on
    // the thread plans every run with the rule stage, although its second
    // run would be served by the incremental index.
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("eins.txt"), b"1").unwrap();
    let (root_a, root_b) = (super::test_support::fwd(&a), super::test_support::fwd(&b));
    let (side_a, side_b) = (LocalBackend::new(&root_a), LocalBackend::new(&root_b));
    let owner = format!("vaulttaskincremental{}", std::process::id());
    let opts = crate::bisync::BisyncOptions {
        direction: crate::bisync::Direction::AtoB,
        delete: crate::bisync::DeletePolicy::Mirror,
        ..Default::default()
    };
    let rules = Arc::new(VaultRules::new(None));
    let installed: Arc<dyn PlanRules> = rules.clone();
    let globs = crate::bisync::empty_globset();
    let filter = crate::bisync::WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    {
        let _rules = plan_rules::enter(installed);
        for round in 1..=3 {
            if round == 3 {
                std::fs::write(a.join("zwei.txt"), b"2").unwrap();
            }
            let outcome = crate::bisync::run_with(crate::bisync::RunRequest {
                settings: RunSettings::for_job(&owner),
                ..crate::bisync::RunRequest::new(
                    &side_a, &root_a, &side_b, &root_b, opts, &filter, &cancel,
                )
            });
            assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
            assert_eq!(rules.adjusted(), round, "round {round} planned in full");
        }
    }
    assert!(b.join("zwei.txt").exists());
    let _ = crate::bisync::forget_job_state(&owner);
    if let Some(log) = crate::bisync::job_log_path(&owner) {
        let _ = std::fs::remove_file(log);
    }

    // Runner level: a one-way mirror vault job plans every run in full.
    let f = Fixture::new();
    f.write(&f.root_a, "/a.txt", b"a");
    let (mut job, site_b) = f.replica_job();
    job.direction = crate::bisync::Direction::AtoB;
    job.delete_policy = crate::bisync::DeletePolicy::Mirror;
    job.validate().expect("one-way mirror vault job");
    for round in 0..3 {
        if round == 2 {
            f.write(&f.root_a, "/b.txt", b"b");
        }
        let outcome = f.run(&job);
        assert!(outcome.unavailable.is_none());
        assert!(outcome.planned_with_rules >= 1, "round {round}");
    }
    assert_eq!(f.tree(&site_b.location), f.tree(&f.root_a));
}

#[test]
fn vault_task_sync_runs_locked() {
    let f = Fixture::new();
    f.write(&f.root_a, "/geheim.txt", b"nur mit Schluessel lesbar");
    let (job, site_b) = f.replica_job();
    // No session of either site exists in the register of this process:
    // the job works on the encrypted storage only (C5.1).
    let register = crate::vault::register();
    assert!(register.session(&f.site_a.id).is_none());
    assert!(register.session(&site_b.id).is_none());
    let outcome = f.run(&job);
    assert!(outcome.unavailable.is_none());
    assert!(
        outcome.outcome.errors.is_empty(),
        "{:?}",
        outcome.outcome.errors
    );
    assert!(register.session(&f.site_a.id).is_none());
    assert!(register.session(&site_b.id).is_none());
    // The copy decrypts with the vault's key afterwards.
    assert_eq!(
        f.tree(&site_b.location)["/geheim.txt"],
        b"nur mit Schluessel lesbar"
    );
    // The storage of B never holds the cleartext.
    for bytes in stored(&site_b.location).values() {
        assert!(!bytes
            .windows(b"Schluessel".len())
            .any(|window| window == b"Schluessel"));
    }
}

/// Local files with server hashes (`content_md5`) and times in whole
/// seconds, like a WebDAV or Drive storage.
struct Hashed {
    inner: LocalBackend,
    identity: String,
}

impl Hashed {
    fn meta(&self, mut meta: VfsMeta, path: &str) -> VfsMeta {
        meta.mtime_ms -= meta.mtime_ms.rem_euclid(1000);
        if !meta.is_dir {
            if let Ok(bytes) = std::fs::read(path) {
                meta.content_md5 = Some(format!("{:x}", md5::compute(bytes)));
            }
        }
        meta
    }
}

impl crate::vfs::Backend for Hashed {
    fn scheme(&self) -> Scheme {
        Scheme::WebDav
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn state_identity(&self) -> String {
        self.identity.clone()
    }
    fn namespace_identity(&self) -> String {
        self.identity.clone()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let entries = self.inner.list_dir(path)?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let child = format!("{}/{}", path.trim_end_matches('/'), entry.name);
                self.meta(entry, &child)
            })
            .collect())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Ok(self.meta(self.inner.stat(path)?, path))
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.inner.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write(path)
    }
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        self.inner.rename(from, to)
    }
    fn rename_no_replace(&self, from: &str, to: &str) -> VfsResult<()> {
        self.inner.rename_no_replace(from, to)
    }
    fn promote_staged(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged(staged, destination)
    }
    fn promote_staged_no_replace(&self, staged: &str, destination: &str) -> VfsResult<()> {
        self.inner.promote_staged_no_replace(staged, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.inner.mkdir_all(path)
    }
    fn create_dir(&self, path: &str) -> VfsResult<()> {
        self.inner.create_dir(path)
    }
    fn create_dir_new(&self, path: &str) -> VfsResult<()> {
        self.inner.create_dir_new(path)
    }
    fn rename_overwrites(&self) -> bool {
        self.inner.rename_overwrites()
    }
    fn staged_write_capabilities(&self, root: &str) -> crate::vfs::StagedWriteCapabilities {
        self.inner.staged_write_capabilities(root)
    }
    fn probe_staged_write_capabilities(
        &self,
        root: &str,
    ) -> VfsResult<crate::vfs::StagedWriteCapabilities> {
        self.inner.probe_staged_write_capabilities(root)
    }
    fn provides_content_hash(&self) -> bool {
        true
    }
}

/// Rewrites `/Bericht.txt` on A with new bytes of the same length and
/// puts its cipher file back to its old time: same size, same second.
fn same_size_same_second(f: &Fixture) {
    let before = stored(&f.root_a);
    let times: Vec<(String, std::time::SystemTime)> = before
        .keys()
        .map(|rel| {
            let path = format!("{}/{rel}", f.root_a);
            (
                rel.clone(),
                std::fs::metadata(&path).unwrap().modified().unwrap(),
            )
        })
        .collect();
    f.write(&f.root_a, "/Bericht.txt", b"Inhalt B");
    let after = stored(&f.root_a);
    let changed: Vec<&String> = after
        .iter()
        .filter(|(rel, bytes)| before.get(*rel).is_some_and(|old| old != *bytes))
        .map(|(rel, _)| rel)
        .collect();
    assert_eq!(changed.len(), 1, "one cipher file changed");
    assert_eq!(after[changed[0]].len(), before[changed[0]].len());
    let (_, time) = times.iter().find(|(rel, _)| rel == changed[0]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(format!("{}/{}", f.root_a, changed[0]))
        .unwrap()
        .set_modified(*time)
        .unwrap();
}

#[test]
fn vault_task_sync_hash_detects_same_size_same_second() {
    let identities = Mutex::new(Vec::<String>::new());
    let hashed = |location: &str| -> Result<(BackendHandle, String), String> {
        identities.lock().unwrap().push(location.to_string());
        let backend = Hashed {
            inner: LocalBackend::new("/"),
            identity: format!("hashed:{location}"),
        };
        Ok((Arc::new(backend), location.to_string()))
    };
    let f = Fixture::new();
    f.write(&f.root_a, "/Bericht.txt", b"Inhalt A");
    let (job, site_b) = f.replica_job();
    assert!(f.run_with(&job, &hashed).outcome.errors.is_empty());
    same_size_same_second(&f);
    let outcome = f.run_with(&job, &hashed);
    assert!(
        outcome.outcome.errors.is_empty(),
        "{:?}",
        outcome.outcome.errors
    );
    assert_eq!(
        outcome.outcome.stats.a_to_b, 1,
        "the server hash shows the change"
    );
    assert_eq!(f.tree(&site_b.location)["/Bericht.txt"], b"Inhalt B");
    assert!(identities.lock().unwrap().len() >= 4);

    // Without server hashes the same change stays unseen (the documented
    // gap of SFTP v3 and FTP, C5.5): the test does change size-equal bytes
    // in the same second.
    let g = Fixture::new();
    g.write(&g.root_a, "/Bericht.txt", b"Inhalt A");
    let (job, site_b) = g.replica_job();
    assert!(g.run(&job).outcome.errors.is_empty());
    same_size_same_second(&g);
    let outcome = g.run(&job);
    assert_eq!(outcome.outcome.stats.a_to_b, 0);
    assert_eq!(g.tree(&site_b.location)["/Bericht.txt"], b"Inhalt A");
}

#[test]
fn vault_task_sync_log_contains_no_cleartext() {
    const MARKER: &str = "KlartextMarkierung7f3";
    let f = Fixture::new();
    f.write(&f.root_a, &format!("/{MARKER}.txt"), MARKER.as_bytes());
    f.write(&f.root_a, &format!("/{MARKER}-Ordner/innen.txt"), b"x");
    let (job, site_b) = f.replica_job();
    let first = f.run(&job);
    assert!(
        first.outcome.errors.is_empty(),
        "{:?}",
        first.outcome.errors
    );
    // A conflict and a change make the log write rels of every kind.
    f.write(&f.root_a, &format!("/{MARKER}.txt"), b"A");
    f.write(&site_b.location, &format!("/{MARKER}.txt"), b"B!");
    f.write(&f.root_a, &format!("/{MARKER}-Ordner/innen.txt"), b"y");
    let second = f.run(&job);
    assert_eq!(second.outcome.conflicts.len(), 1);
    let log = crate::bisync::read_job_log(&job.id, None).expect("job log");
    assert!(!log.text.is_empty(), "the run wrote its log");
    assert!(!log.text.contains(MARKER), "cleartext name in the job log");
    let texts = [first, second]
        .iter()
        .flat_map(|run| {
            let outcome = &run.outcome;
            outcome
                .errors
                .iter()
                .map(|(rel, text)| format!("{rel} {text}"))
                .chain(
                    outcome
                        .deferred
                        .iter()
                        .map(|(rel, text)| format!("{rel} {text}")),
                )
                .chain(outcome.conflicts.iter().map(|item| item.rel.clone()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(texts.iter().all(|text| !text.contains(MARKER)), "{texts:?}");
    for bytes in stored(&site_b.location).values() {
        assert!(!bytes
            .windows(MARKER.len())
            .any(|window| window == MARKER.as_bytes()));
    }
}
