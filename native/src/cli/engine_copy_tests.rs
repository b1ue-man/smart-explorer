//! Acceptance signals of the engine path of `se cp`/`get`/`put` and the MCP
//! `copy` (U49). Every test passes the switch explicitly, so none depends on
//! `CLI_ENGINE_PATH`.
use super::engine_copy::{eligible, settle, try_copy_with, Eligibility, Finished};
use super::entry_kind::EntryKind;
use super::target::Target;
use super::tree_observer::{TreeObserver, TreePhase, TRANSFER_CANCELLED};
use super::tree_ops::TreeOutcome;
use crate::transfer::walk::{omission_kind, OmissionKind};
use crate::transfer::{TransferIssue, TransferKind, TransferProgress};
use crate::vfs::{BackendHandle, LocalBackend};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

const LINK_TEXT: &str = "Links, Reparse-Punkte und Spezialdateien werden nicht übertragen";
const SPECIAL_TEXT: &str = "Spezialdatei wird nicht übertragen";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let unique = format!(
            "speed-task-cli-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create scratch folder");
        Self(path)
    }

    fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().replace('\\', "/")
    }

    fn os(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn dir(&self, rel: &str) -> String {
        fs::create_dir_all(self.os(rel)).expect("create folder");
        self.path(rel)
    }

    fn file(&self, rel: &str, bytes: &[u8]) -> String {
        if let Some(parent) = self.os(rel).parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(self.os(rel), bytes).expect("write file");
        self.path(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target(path: &str) -> Target {
    let backend: BackendHandle = Arc::new(LocalBackend::new("/"));
    Target::with_backend_key(backend, path.to_string(), "local")
}

fn copy_with(
    enabled: bool,
    src: &str,
    dst: &str,
    recursive: bool,
    observer: &TreeObserver,
) -> Result<Option<TreeOutcome>, String> {
    let (source, destination) = (target(src), target(dst));
    let meta = source.backend.stat(&source.path).expect("stat source");
    try_copy_with(
        enabled,
        &source,
        &meta,
        &meta,
        &destination,
        dst,
        recursive,
        observer,
    )
}

fn facts(source: EntryKind) -> Eligibility {
    Eligibility {
        enabled: true,
        source,
        recursive: false,
        destination_absent: true,
        parent_is_plain_dir: true,
        disjoint: true,
        name_ok: true,
    }
}

fn issue(path: &str, message: &str) -> TransferIssue {
    TransferIssue {
        path: path.to_string(),
        message: message.to_string(),
    }
}

fn finished(issues: Vec<TransferIssue>) -> Finished {
    let mut progress = TransferProgress::new(TransferKind::Local, "copy", 0, 0);
    progress.done = true;
    progress.errors = issues.len() as u64;
    Finished {
        progress,
        canceled: false,
        issues,
        roots: Vec::new(),
    }
}

#[path = "engine_copy_settle_tests.rs"]
mod settle;

#[test]
fn speed_task_cli_engine_eligible_requires_absent_destination_and_existing_parent() {
    assert!(eligible(&facts(EntryKind::File)));
    let mut present = facts(EntryKind::File);
    present.destination_absent = false;
    assert!(!eligible(&present));
    let mut no_parent = facts(EntryKind::File);
    no_parent.parent_is_plain_dir = false;
    assert!(!eligible(&no_parent));
    let mut inside = facts(EntryKind::File);
    inside.disjoint = false;
    assert!(!eligible(&inside));
    let mut odd_name = facts(EntryKind::File);
    odd_name.name_ok = false;
    assert!(!eligible(&odd_name));
    let mut off = facts(EntryKind::File);
    off.enabled = false;
    assert!(!eligible(&off));

    // The same rule against the real file system.
    let scratch = Scratch::new("eligible");
    let src = scratch.file("src.txt", b"data");
    let existing = scratch.file("existing.txt", b"old");
    let observer = TreeObserver::default();
    assert!(copy_with(true, &src, &existing, false, &observer)
        .expect("fallback is no error")
        .is_none());
    assert_eq!(fs::read(scratch.os("existing.txt")).expect("read"), b"old");
    let orphan = scratch.path("missing-parent/new.txt");
    assert!(copy_with(true, &src, &orphan, false, &observer)
        .expect("fallback is no error")
        .is_none());
    assert!(!scratch.os("missing-parent").exists());
    let fresh = scratch.path("fresh.txt");
    assert!(copy_with(false, &src, &fresh, false, &observer)
        .expect("disabled is no error")
        .is_none());
    assert!(!scratch.os("fresh.txt").exists());
    // `lstat`: a dangling link is a present destination, not a missing one.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(scratch.os("nowhere"), scratch.os("dangling")).expect("link");
        let dangling = scratch.path("dangling");
        assert!(copy_with(true, &src, &dangling, false, &observer)
            .expect("fallback is no error")
            .is_none());
        assert!(fs::symlink_metadata(scratch.os("dangling")).is_ok());
        assert!(!scratch.os("nowhere").exists());
    }
}

#[test]
fn speed_task_cli_engine_eligible_rejects_link_roots_and_unrecursive_directories() {
    assert!(!eligible(&facts(EntryKind::Link)));
    assert!(!eligible(&facts(EntryKind::Special)));
    assert!(!eligible(&facts(EntryKind::Dir)));
    let mut recursive = facts(EntryKind::Dir);
    recursive.recursive = true;
    assert!(eligible(&recursive));

    let scratch = Scratch::new("rejects");
    let folder = scratch.dir("folder");
    scratch.file("folder/child.txt", b"child");
    let observer = TreeObserver::default();
    let target_path = scratch.path("copy");
    assert!(copy_with(true, &folder, &target_path, false, &observer)
        .expect("fallback is no error")
        .is_none());
    assert!(!scratch.os("copy").exists());
    #[cfg(unix)]
    {
        let real = scratch.file("real.txt", b"real");
        std::os::unix::fs::symlink(&real, scratch.os("link.txt")).expect("link");
        let link = scratch.path("link.txt");
        let linked = scratch.path("linked-copy.txt");
        assert!(copy_with(true, &link, &linked, false, &observer)
            .expect("fallback is no error")
            .is_none());
        assert!(!scratch.os("linked-copy.txt").exists());
    }
}

#[test]
fn speed_task_cli_engine_copies_a_file_to_a_new_name() {
    let scratch = Scratch::new("file");
    let content: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    let src = scratch.file("source-name.bin", &content);
    let dst = scratch.path("typed-name.dat");
    let observer = TreeObserver::default();
    let outcome = copy_with(true, &src, &dst, false, &observer)
        .expect("copy")
        .expect("on the engine");
    assert_eq!(outcome.destination, dst);
    assert!(outcome.omitted.is_empty() && outcome.kept.is_empty());
    assert_eq!(
        fs::read(scratch.os("typed-name.dat")).expect("read"),
        content
    );
    assert_eq!(
        fs::read(scratch.os("source-name.bin")).expect("read"),
        content
    );
    let mut names: Vec<String> = fs::read_dir(&scratch.0)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["source-name.bin", "typed-name.dat"]);
    let progress = observer.progress();
    assert!(progress.engine);
    assert_eq!(progress.phase, TreePhase::Done);
    assert_eq!(progress.files_written, 1);
    assert_eq!(progress.bytes_written, content.len() as u64);
}

#[test]
fn speed_task_cli_engine_copies_a_tree_to_a_new_name_with_children_as_roots() {
    let scratch = Scratch::new("tree");
    let src = scratch.dir("original");
    scratch.file("original/a.txt", b"a");
    scratch.file("original/sub/b.txt", b"bb");
    scratch.file("original/sub/deeper/c.txt", b"ccc");
    scratch.dir("original/empty");
    let dst = scratch.path("renamed");
    let observer = TreeObserver::default();
    let outcome = copy_with(true, &src, &dst, true, &observer)
        .expect("copy")
        .expect("on the engine");
    assert_eq!(outcome.destination, dst);
    assert_eq!(fs::read(scratch.os("renamed/a.txt")).expect("a"), b"a");
    assert_eq!(fs::read(scratch.os("renamed/sub/b.txt")).expect("b"), b"bb");
    assert_eq!(
        fs::read(scratch.os("renamed/sub/deeper/c.txt")).expect("c"),
        b"ccc"
    );
    assert!(scratch.os("renamed/empty").is_dir());
    // The typed name is the root: the source name does not appear below it.
    assert!(!scratch.os("renamed/original").exists());
    let progress = observer.progress();
    assert_eq!(progress.files_written, 3);
    assert_eq!(progress.bytes_written, 6);
}

#[test]
fn speed_task_cli_engine_empty_directory_is_created() {
    let scratch = Scratch::new("empty");
    let src = scratch.dir("nothing");
    let dst = scratch.path("made");
    let observer = TreeObserver::default();
    let outcome = copy_with(true, &src, &dst, true, &observer)
        .expect("copy")
        .expect("on the engine");
    assert_eq!(outcome.destination, dst);
    assert!(scratch.os("made").is_dir());
    assert_eq!(fs::read_dir(scratch.os("made")).expect("list").count(), 0);
    let progress = observer.progress();
    assert!(progress.engine);
    assert_eq!(progress.phase, TreePhase::Done);
    assert_eq!(progress.files_written, 0);
}

#[test]
fn speed_task_cli_engine_falls_back_for_an_existing_destination() {
    let scratch = Scratch::new("fallback");
    let src_file = scratch.file("f.txt", b"new");
    let src_dir = scratch.dir("d");
    scratch.file("d/child.txt", b"child");
    let dst_file = scratch.file("taken.txt", b"old");
    let dst_dir = scratch.dir("taken-dir");
    let observer = TreeObserver::default();
    assert!(copy_with(true, &src_file, &dst_file, false, &observer)
        .expect("no error")
        .is_none());
    assert!(copy_with(true, &src_dir, &dst_dir, true, &observer)
        .expect("no error")
        .is_none());
    assert_eq!(fs::read(scratch.os("taken.txt")).expect("read"), b"old");
    assert_eq!(
        fs::read_dir(scratch.os("taken-dir")).expect("list").count(),
        0
    );
    // Nothing ran on the engine.
    assert!(!observer.progress().engine);
}

#[test]
fn speed_task_cli_engine_reports_omissions_with_kinds() {
    assert_eq!(omission_kind(LINK_TEXT), Some(OmissionKind::Link));
    assert_eq!(omission_kind(SPECIAL_TEXT), Some(OmissionKind::Special));
    assert_eq!(
        omission_kind(&format!("keine reguläre Datei ({LINK_TEXT})")),
        Some(OmissionKind::Other)
    );
    assert_eq!(omission_kind("Zugriff verweigert"), None);

    let run = finished(vec![
        issue("/src/link", LINK_TEXT),
        issue("/src/fifo", SPECIAL_TEXT),
    ]);
    let outcome = settle(&run, "/dst").expect("omissions are no failure");
    assert_eq!(outcome.destination, "/dst");
    let kinds: Vec<(&str, EntryKind)> = outcome
        .omitted
        .iter()
        .map(|omission| (omission.path.as_str(), omission.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("/src/link", EntryKind::Link),
            ("/src/fifo", EntryKind::Special)
        ]
    );
    // The notices read as they do on the older path.
    let notices = outcome.notices();
    assert!(notices[0].starts_with("skipped link"));
    assert!(notices[1].starts_with("skipped special file"));

    // A link inside a real tree on the engine.
    #[cfg(unix)]
    {
        let scratch = Scratch::new("omissions");
        let src = scratch.dir("tree");
        scratch.file("tree/real.txt", b"real");
        std::os::unix::fs::symlink(scratch.os("tree/real.txt"), scratch.os("tree/alias"))
            .expect("link");
        let dst = scratch.path("tree-copy");
        let observer = TreeObserver::default();
        let outcome = copy_with(true, &src, &dst, true, &observer)
            .expect("copy")
            .expect("on the engine");
        assert_eq!(
            fs::read(scratch.os("tree-copy/real.txt")).expect("real"),
            b"real"
        );
        assert!(!scratch.os("tree-copy/alias").exists());
        assert_eq!(outcome.omitted.len(), 1);
        assert_eq!(outcome.omitted[0].kind, EntryKind::Link);
        assert_eq!(outcome.omitted[0].path, scratch.path("tree/alias"));
    }
}

#[test]
fn speed_task_cli_engine_cancel_stops_the_run() {
    let mut run = finished(Vec::new());
    run.canceled = true;
    assert_eq!(settle(&run, "/dst").unwrap_err(), TRANSFER_CANCELLED);

    let scratch = Scratch::new("cancel");
    let src = scratch.file("big.bin", &vec![7u8; 200_000]);
    let dst = scratch.path("never.bin");
    let observer = TreeObserver::default();
    observer.cancel();
    let error = copy_with(true, &src, &dst, false, &observer).unwrap_err();
    assert_eq!(error, TRANSFER_CANCELLED);
}

#[test]
fn speed_task_cli_engine_source_change_before_start_is_refused() {
    let scratch = Scratch::new("changed");
    let src = scratch.file("source.txt", b"planned");
    let dst = scratch.path("copy.txt");
    let (source, destination) = (target(&src), target(&dst));
    let planned = source.backend.stat(&source.path).expect("stat");
    // The file grows between the plan and the start of the transfer.
    fs::write(scratch.os("source.txt"), b"planned and then some").expect("rewrite");
    let observer = TreeObserver::default();
    let error = try_copy_with(
        true,
        &source,
        &planned,
        &planned,
        &destination,
        &dst,
        false,
        &observer,
    )
    .unwrap_err();
    assert!(
        error.contains("source type, identity, or size changed"),
        "{error}"
    );
    assert!(!scratch.os("copy.txt").exists());
}
