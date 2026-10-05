//! Test-only Drive endpoints shared by the one reliability task. The HTTP
//! transport, resolver-facing handle, private registry and sync engine are real.
use super::api::FOLDER_MIME;
use super::sync_conflict_task_fixture::{filter, MIME};
use super::task_drive::FakeDrive;
use super::task_http::{Answer, Request, Server};
use super::GDriveBackend;
use crate::bisync::{self, BisyncOptions, CompareMode, Direction, Outcome, StateKey};
use crate::vfs::{self, Backend, BackendHandle, LocalBackend};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct DriveFixture {
    pub(super) drive: Arc<FakeDrive>,
    pub(super) server: Server,
    pub(super) backend: GDriveBackend,
    pub(super) local: LocalBackend,
    pub(super) local_root: String,
    pub(super) root: String,
    pub(super) directory: tempfile::TempDir,
    next: AtomicUsize,
}

impl DriveFixture {
    pub(crate) fn new(root: &str) -> Self {
        Self::with_handler(root, |_, _| None)
    }

    pub(super) fn with_handler(
        root: &str,
        handler: impl Fn(&FakeDrive, &Request) -> Option<Answer> + Send + Sync + 'static,
    ) -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let serial = SERIAL.fetch_add(1, Ordering::SeqCst);
        let directory = tempfile::tempdir().unwrap();
        let local_path = directory.path().join("local");
        std::fs::create_dir(&local_path).unwrap();
        let local_root = local_path.to_string_lossy().replace('\\', "/");
        let drive = Arc::new(FakeDrive::new(
            &format!("reliability-root-{serial}"),
            &format!("reliability-user-{serial}"),
        ));
        drive.set_page_size(2);
        let mut parent = drive.root_id.clone();
        for (index, name) in root.split('/').filter(|name| !name.is_empty()).enumerate() {
            let id = format!("fixture-root-{serial}-{index}");
            drive.insert(&id, name, &parent, FOLDER_MIME, b"");
            parent = id;
        }
        let server = {
            let drive = drive.clone();
            Server::start(move |request| {
                handler(&drive, request).unwrap_or_else(|| drive.answer(request))
            })
        };
        let backend = Self::open(&server, &directory, &drive.permission_id, root);
        let local = LocalBackend::new(&local_root);
        Self {
            drive,
            server,
            backend,
            local,
            local_root,
            root: format!("/{}", root.trim_matches('/')),
            directory,
            next: AtomicUsize::new(0),
        }
    }

    fn open(
        server: &Server,
        directory: &tempfile::TempDir,
        account: &str,
        root: &str,
    ) -> GDriveBackend {
        let backend = GDriveBackend::test_backend_with_identity(
            &server.api_base(),
            Duration::from_secs(2),
            Some(directory.path().join("pending")),
            Some(directory.path().join("bindings")),
            account,
            root,
        );
        // A refresh credential belongs to one account. The generic factory's
        // shared token would make unrelated accounts claim the same legacy
        // state identity, which the real pair owner correctly rejects.
        backend.tokens_guard().unwrap().refresh_token = format!("reliability-refresh-{account}");
        backend
    }

    pub(super) fn fresh_backend(&self, root: &str) -> GDriveBackend {
        Self::open(
            &self.server,
            &self.directory,
            &self.drive.permission_id,
            root,
        )
    }

    pub(super) fn historical_cache_backend(
        &self,
        root: &str,
        key: &str,
        id: &str,
    ) -> GDriveBackend {
        let legacy = self.directory.path().join("legacy-path-cache.json");
        let account = self.directory.path().join("account-path-cache.json");
        super::cache::save_to_path(
            &legacy,
            std::collections::HashMap::from([(key.to_string(), id.to_string())]),
            std::collections::HashMap::from([(key.to_string(), FOLDER_MIME.to_string())]),
        )
        .unwrap();
        self.fresh_backend(root)
            .test_with_loaded_caches(account, &legacy)
    }

    pub(crate) fn endpoint(&self) -> (BackendHandle, String) {
        (Arc::new(self.backend.clone()), self.root.clone())
    }

    pub(crate) fn restart(&self) -> BackendHandle {
        Arc::new(self.fresh_backend(&self.root))
    }

    /// Seed/change an ordinary path in this fixture's own tree. Changing it
    /// preserves its exact ID; the subsequent test uses normal HTTP calls.
    pub(crate) fn write_file(&self, rel: &str, bytes: &[u8]) -> String {
        let mut components = rel.split('/').collect::<Vec<_>>();
        let title = components.pop().expect("fixture needs a filename");
        assert!(!title.is_empty());
        let mut parent = self.root_object_id();
        for name in components {
            let existing = self.drive.named(&parent, name);
            parent = if existing.is_empty() {
                let id = self.id();
                self.drive.insert(&id, name, &parent, FOLDER_MIME, b"");
                id
            } else {
                assert_eq!(existing.len(), 1);
                assert_eq!(existing[0]["mimeType"], FOLDER_MIME);
                existing[0]["id"].as_str().unwrap().into()
            };
        }
        let existing = self.drive.named(&parent, title);
        assert!(
            existing.len() <= 1,
            "ordinary fixture writes never pick a duplicate"
        );
        let id = existing
            .first()
            .and_then(|file| file["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| self.id());
        self.drive.insert(&id, title, &parent, MIME, bytes);
        id
    }

    fn id(&self) -> String {
        format!(
            "{}-seed-{}",
            self.drive.root_id,
            self.next.fetch_add(1, Ordering::SeqCst)
        )
    }

    pub(super) fn root_object_id(&self) -> String {
        let mut parent = self.drive.root_id.clone();
        for name in self.root.split('/').filter(|name| !name.is_empty()) {
            let objects = self.drive.named(&parent, name);
            assert_eq!(objects.len(), 1);
            parent = objects[0]["id"].as_str().unwrap().into();
        }
        parent
    }

    pub(crate) fn read_file(&self, rel: &str) -> Vec<u8> {
        read(
            &self.backend,
            &vfs::sync_path(&self.backend, &self.root, rel).unwrap(),
        )
    }

    pub(super) fn local_bytes(&self, rel: &str) -> Vec<u8> {
        std::fs::read(Path::new(&self.local_root).join(rel)).unwrap()
    }

    pub(super) fn write_local(&self, rel: &str, bytes: &[u8]) {
        let path = Path::new(&self.local_root).join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    pub(super) fn preview(&self, opts: BisyncOptions) -> bisync::Preview {
        bisync::preview(
            &self.local,
            &self.local_root,
            &self.backend,
            &self.root,
            opts,
            &AtomicBool::new(false),
            &filter(&bisync::empty_globset()),
        )
    }

    pub(super) fn run(&self, opts: BisyncOptions) -> Outcome {
        self.run_cancel(opts, &AtomicBool::new(false))
    }

    pub(super) fn run_cancel(&self, opts: BisyncOptions, cancel: &AtomicBool) -> Outcome {
        bisync::run(
            &self.local,
            &self.local_root,
            &self.backend,
            &self.root,
            opts,
            cancel,
            &filter(&bisync::empty_globset()),
        )
    }

    pub(super) fn mutations(&self) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|request| request.method != "GET")
            .count()
    }

    pub(super) fn mutations_to(&self, id: &str) -> usize {
        let suffix = format!("/files/{id}");
        self.server
            .requests()
            .iter()
            .filter(|request| request.method != "GET" && request.path().ends_with(&suffix))
            .count()
    }

    pub(super) fn folder_posts(&self) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|request| request.method == "POST" && request.path() == "/drive/v3/files")
            .count()
    }

    pub(super) fn aliases(&self, path: &str) -> BTreeMap<String, String> {
        vfs::list_dir_tolerant(&self.backend, path)
            .unwrap()
            .entries
            .into_iter()
            .filter(|entry| entry.is_dir)
            .map(|entry| (entry.id.unwrap(), entry.name))
            .collect()
    }

    pub(super) fn registry_bytes(&self) -> BTreeMap<String, Vec<u8>> {
        collect_bytes(&self.directory.path().join("bindings"), true)
    }

    /// The .171 fixture predates the new folder registry. Remove only this
    /// fixture's post-upgrade records, retaining its real pair state and data.
    pub(super) fn restore_pre_registry_state(&self) {
        let root = self.directory.path().join("bindings");
        assert_eq!(root.parent(), Some(self.directory.path()));
        assert!(!self.registry_bytes().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
        assert!(!root.exists());
    }

    pub(super) fn assert_noop(&self, opts: BisyncOptions, expected: &Outcome) -> Outcome {
        let mutations = self.mutations();
        let next = self.run(opts);
        assert_complete(&next);
        assert_eq!(next.stats.bytes, 0);
        assert_eq!(
            next.stats.a_to_b + next.stats.b_to_a + next.stats.deleted,
            0
        );
        assert_eq!(next.baseline, expected.baseline);
        assert_eq!(self.mutations(), mutations);
        assert_persisted(&next);
        next
    }
}

pub(super) fn options(direction: Direction) -> BisyncOptions {
    BisyncOptions {
        direction,
        compare: CompareMode::Checksum,
        max_transfers: 1,
        ..Default::default()
    }
}

pub(super) fn run_backends(
    left: &dyn Backend,
    left_root: &str,
    right: &dyn Backend,
    right_root: &str,
    direction: Direction,
) -> bisync::Outcome {
    bisync::run(
        left,
        left_root,
        right,
        right_root,
        options(direction),
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    )
}

pub(super) fn assert_complete_at(out: &bisync::Outcome, phase: &str) {
    eprintln!("C02 Drive roundtrip phase={phase}");
    assert_complete(out);
}

pub(super) fn assert_complete(out: &Outcome) {
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(
        out.conflicts.is_empty(),
        "unresolved conflicts: {}",
        out.conflicts.len()
    );
    assert!(
        out.omissions.is_empty(),
        "protected omissions are a partial result: {:?}",
        out.omissions
    );
    assert!(out.blocked.is_none() && out.stopped.is_none() && !out.busy && !out.canceled);
    assert!(out.deferred.is_empty(), "deferred files are not converged");
}

pub(super) fn assert_persisted(out: &Outcome) {
    let path = bisync::baseline_file(out.state.as_ref().expect("run has StateKey")).unwrap();
    assert_eq!(bisync::load_baseline(&path).unwrap(), out.baseline);
}

pub(super) fn read(backend: &dyn Backend, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    backend
        .open_read(path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    bytes
}

pub(super) fn contains_bytes(path: &Path, expected: &[u8]) -> bool {
    collect_bytes(path, false)
        .values()
        .any(|bytes| bytes == expected)
}

fn collect_bytes(root: &Path, json_only: bool) -> BTreeMap<String, Vec<u8>> {
    fn visit(path: &Path, base: &Path, json_only: bool, files: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, base, json_only, files);
            } else if !json_only || path.extension().is_some_and(|ext| ext == "json") {
                files.insert(
                    path.strip_prefix(base).unwrap().to_string_lossy().into(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, json_only, &mut files);
    files
}

pub(super) fn state_baseline(key: &StateKey) -> bisync::Baseline {
    bisync::load_baseline(&bisync::baseline_file(key).unwrap()).unwrap()
}

/// Reconstruct the former folded pair from real seed signatures and side
/// spellings. The caller removed only the extra exact-case seed copies.
pub(super) fn legacy_folded_state(
    seed: &Outcome,
    rel_a: &str,
    rel_b: &str,
) -> bisync::Baseline {
    let path = bisync::baseline_file(seed.state.as_ref().unwrap()).unwrap();
    let mut baseline = seed.baseline.clone();
    let a = baseline[rel_a].0;
    let b = baseline[rel_b].1;
    assert!(a.is_some() && b.is_some());
    if rel_a != rel_b {
        baseline.remove(rel_b);
    }
    baseline.insert(rel_a.into(), (a, b));
    let spelling_path = path.with_extension("spellings.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&spelling_path).unwrap()).unwrap();
    let mut legacy = serde_json::Map::new();
    for field in ["files_a", "files_b", "dirs_a", "dirs_b"] {
        let entries = value[field].as_object_mut().unwrap();
        if rel_a != rel_b && field.ends_with("_b") {
            entries.remove(rel_a);
            entries.remove(rel_a.rsplit_once('/').unwrap().0);
        } else if rel_a != rel_b {
            entries.remove(rel_b);
            entries.remove(rel_b.rsplit_once('/').unwrap().0);
        }
        let mut folded = serde_json::Map::new();
        for (key, rel) in std::mem::take(entries) {
            let name = rel.as_str().unwrap();
            assert!(name.is_ascii()); // exactly the prior KeyPolicy for these fixtures
            assert_eq!(key, name);
            assert!(folded.insert(name.to_uppercase(), rel).is_none());
        }
        legacy.insert(field.to_string(), serde_json::Value::Object(folded));
    }
    std::fs::write(&spelling_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
    let dirs_path = path.with_extension("dirs.json");
    let mut dirs: BTreeSet<String> =
        serde_json::from_slice(&std::fs::read(&dirs_path).unwrap()).unwrap();
    if rel_a != rel_b {
        // The completed old folded pair had one common A-spelled dir record,
        // while the modern exact seed had both independently copied trees.
        assert!(dirs.remove(rel_b.rsplit_once('/').unwrap().0));
        assert!(dirs.contains(rel_a.rsplit_once('/').unwrap().0));
    }
    std::fs::write(&dirs_path, serde_json::to_vec(&dirs).unwrap()).unwrap();
    assert!(!path.with_extension("journal").exists());
    bisync::save_baseline(&path, &baseline).unwrap();
    baseline
}

/// Selecting an exact root can start a new pair with retained local bytes.
/// Preserve the normal strict file-conflict flow and explicitly choose the
/// selected Drive copy if that first preview reports the existing note.
pub(super) fn resolve_initial_note(
    f: &DriveFixture,
    backend: &GDriveBackend,
    root: &str,
    opts: BisyncOptions,
) {
    let preview = bisync::preview(
        &f.local,
        &f.local_root,
        backend,
        root,
        opts,
        &AtomicBool::new(false),
        &filter(&bisync::empty_globset()),
    );
    assert!(preview.error.is_none() && preview.blocked.is_none());
    let state = preview.state.unwrap();
    for conflict in &preview.conflicts {
        assert_eq!(conflict.rel, "note.md");
        assert!(conflict.duplicates.is_none());
        bisync::resolve_recorded(
            &f.local,
            &f.local_root,
            backend,
            root,
            conflict,
            false,
            None,
            &state,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
    }
}
