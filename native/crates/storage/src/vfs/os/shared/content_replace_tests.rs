//! "Replace in Files" through the storage interface on a local folder: the
//! files the search finds are changed and published whole, write-protected,
//! binary and changed files stay with their reason, the originals are kept
//! encrypted and come back with undo, a dry run writes nothing, a cancel and
//! a failed backup leave the files as they are.
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::agent_proto::{ContentQuery, TextMode};
use crate::vfs::{
    content_replace_with, BackendHandle, HitRef, LocalBackend, ReplaceEvent, ReplaceJournal,
    ReplaceRequest, ReplaceRunState, ReplaceSelection, ReplaceSkip, ReplaceSummary,
};

struct Fixture {
    _dir: tempfile::TempDir,
    root: String,
    journal: ReplaceJournal,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "Hallo Welt\nWelt!\n").unwrap();
    std::fs::write(root.join("sub").join("b.txt"), "keine Treffer\n").unwrap();
    std::fs::write(root.join("sub").join("c.txt"), "noch eine Welt\n").unwrap();
    std::fs::write(root.join("bild.bin"), b"\0\x01Welt").unwrap();
    let journal = ReplaceJournal::open_at(dir.path().join("journal"), [7u8; 32]);
    Fixture {
        root: root.to_string_lossy().replace('\\', "/"),
        _dir: dir,
        journal,
    }
}

fn request(pattern: &str, replacement: &str, selection: ReplaceSelection) -> ReplaceRequest {
    let mut query = ContentQuery::text(pattern);
    query.match_case = true;
    ReplaceRequest {
        query,
        replacement: replacement.to_string(),
        selection,
        dry_run: false,
    }
}

fn run(
    fixture: &Fixture,
    journal: &ReplaceJournal,
    request: &ReplaceRequest,
    cancel: bool,
) -> (ReplaceSummary, Vec<ReplaceEvent>) {
    let backend = LocalBackend::new(&fixture.root);
    let events = Mutex::new(Vec::new());
    let emit = |event: ReplaceEvent| {
        events.lock().unwrap().push(event);
        true
    };
    let cancel = AtomicBool::new(cancel);
    let summary = content_replace_with(
        journal,
        &backend,
        &fixture.root,
        request,
        &fixture.root,
        &emit,
        &cancel,
    )
    .unwrap();
    (summary, events.into_inner().unwrap())
}

fn read(fixture: &Fixture, rel: &str) -> String {
    std::fs::read_to_string(Path::new(&fixture.root).join(rel)).unwrap()
}

fn skips(events: &[ReplaceEvent]) -> Vec<(String, ReplaceSkip)> {
    let mut skips: Vec<(String, ReplaceSkip)> = events
        .iter()
        .filter_map(|event| match event {
            ReplaceEvent::Skipped { rel, reason } => Some((rel.clone(), reason.clone())),
            _ => None,
        })
        .collect();
    skips.sort_by(|left, right| left.0.cmp(&right.0));
    skips
}

fn set_read_only(path: &Path, read_only: bool) {
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(read_only);
    std::fs::set_permissions(path, permissions).unwrap();
}

/// Every match of every found file; write-protected and binary files stay;
/// the journal holds only ciphertext; undo brings the originals back.
#[test]
fn replace_task_replaces_found_files_and_undoes_them() {
    let fixture = fixture();
    let protected = Path::new(&fixture.root).join("geschützt.txt");
    std::fs::write(&protected, "Welt\n").unwrap();
    set_read_only(&protected, true);
    let (summary, events) = run(
        &fixture,
        &fixture.journal,
        &request("Welt", "Erde", ReplaceSelection::All),
        false,
    );
    set_read_only(&protected, false);
    assert_eq!(
        (summary.files_changed, summary.replacements),
        (2, 3),
        "{events:?}"
    );
    assert!(!summary.canceled && summary.files_searched >= 3);
    assert_eq!(read(&fixture, "a.txt"), "Hallo Erde\nErde!\n");
    assert_eq!(read(&fixture, "sub/c.txt"), "noch eine Erde\n");
    assert_eq!(read(&fixture, "geschützt.txt"), "Welt\n");
    assert_eq!(
        skips(&events),
        [("geschützt.txt".to_string(), ReplaceSkip::ReadOnly)]
    );
    assert_eq!(
        std::fs::read(Path::new(&fixture.root).join("bild.bin")).unwrap(),
        b"\0\x01Welt"
    );
    let id = summary.undo_id.clone().expect("undo id");
    let history = fixture.journal.history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        (history[0].id.as_str(), history[0].state),
        (id.as_str(), ReplaceRunState::Applied)
    );
    assert_eq!((history[0].files_changed, history[0].replacements), (2, 3));
    assert_eq!(history[0].pattern, "Welt");
    // No stage next to the files, no plaintext in the journal.
    let names: Vec<String> = std::fs::read_dir(&fixture.root)
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|name| !name.contains(".se-")), "{names:?}");
    let run_dir = fixture._dir.path().join("journal").join(&id);
    for item in std::fs::read_dir(&run_dir).unwrap() {
        let bytes = std::fs::read(item.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("Hallo") && !text.contains("Welt"),
            "plaintext in journal"
        );
    }
    let backend: BackendHandle = Arc::new(LocalBackend::new(&fixture.root));
    let undone = fixture.journal.undo(&id, &|_| Ok(backend.clone())).unwrap();
    let mut restored = undone.restored.clone();
    restored.sort();
    assert_eq!(restored, ["a.txt", "sub/c.txt"]);
    assert!(undone.skipped.is_empty(), "{:?}", undone.skipped);
    assert_eq!(read(&fixture, "a.txt"), "Hallo Welt\nWelt!\n");
    assert_eq!(read(&fixture, "sub/c.txt"), "noch eine Welt\n");
    assert_eq!(
        fixture.journal.history().unwrap()[0].state,
        ReplaceRunState::Undone
    );
    fixture.journal.discard(&id).unwrap();
    assert!(fixture.journal.history().unwrap().is_empty());
    assert!(fixture.journal.discard(&id).is_err());
    assert!(fixture.journal.discard("../x").is_err());
}

/// Undo leaves a file changed since the replacement, with the reason.
#[test]
fn replace_task_undo_skips_files_changed_since() {
    let fixture = fixture();
    let (summary, _) = run(
        &fixture,
        &fixture.journal,
        &request("Welt", "Erde", ReplaceSelection::All),
        false,
    );
    let id = summary.undo_id.unwrap();
    std::fs::write(
        Path::new(&fixture.root).join("a.txt"),
        "von Hand geändert\n",
    )
    .unwrap();
    let backend: BackendHandle = Arc::new(LocalBackend::new(&fixture.root));
    let undone = fixture.journal.undo(&id, &|_| Ok(backend.clone())).unwrap();
    assert_eq!(undone.restored, ["sub/c.txt"]);
    assert_eq!(undone.skipped.len(), 1);
    assert_eq!(undone.skipped[0].0, "a.txt");
    assert!(
        undone.skipped[0].1.contains("geändert"),
        "{:?}",
        undone.skipped
    );
    assert_eq!(read(&fixture, "a.txt"), "von Hand geändert\n");
    assert_eq!(
        fixture.journal.history().unwrap()[0].state,
        ReplaceRunState::PartlyUndone
    );
}

/// Selected hits and files only; a file whose state differs from the hit's
/// is skipped as changed since the search.
#[test]
fn replace_task_selection_and_changed_files() {
    let fixture = fixture();
    let a = std::fs::metadata(Path::new(&fixture.root).join("a.txt")).unwrap();
    let mtime_ms = a
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let hit = |rel: &str, line, mtime_ms| HitRef {
        rel: rel.to_string(),
        line,
        ranges: vec![(0, 4)],
        size: if rel == "a.txt" { a.len() } else { 15 },
        mtime_ms,
    };
    let selection = ReplaceSelection::Hits(vec![hit("a.txt", 2, mtime_ms), hit("sub/c.txt", 1, 1)]);
    let (summary, events) = run(
        &fixture,
        &fixture.journal,
        &request("Welt", "Erde", selection),
        false,
    );
    assert_eq!(
        (summary.files_changed, summary.replacements),
        (1, 1),
        "{events:?}"
    );
    assert_eq!(read(&fixture, "a.txt"), "Hallo Welt\nErde!\n");
    assert_eq!(read(&fixture, "sub/c.txt"), "noch eine Welt\n");
    assert_eq!(
        skips(&events),
        [("sub/c.txt".to_string(), ReplaceSkip::Changed)]
    );
    let files = ReplaceSelection::Files(vec!["sub/c.txt".into(), "../a.txt".into()]);
    let (summary, events) = run(
        &fixture,
        &fixture.journal,
        &request("Welt", "Erde", files),
        false,
    );
    assert_eq!((summary.files_changed, summary.files_searched), (1, 1));
    assert_eq!(read(&fixture, "sub/c.txt"), "noch eine Erde\n");
    assert_eq!(
        skips(&events),
        [("../a.txt".to_string(), ReplaceSkip::NotRegular)]
    );
}

/// A dry run reports the lines before and after and changes nothing; a
/// cancel before the first file changes nothing; a journal that cannot be
/// written leaves every file unchanged.
#[test]
fn replace_task_dry_run_cancel_and_failed_backup_change_nothing() {
    let fixture = fixture();
    let mut dry = request("Welt", "Erde", ReplaceSelection::All);
    dry.dry_run = true;
    let (summary, events) = run(&fixture, &fixture.journal, &dry, false);
    assert_eq!((summary.files_changed, summary.replacements), (2, 3));
    assert_eq!(summary.undo_id, None);
    let mut previews: Vec<(String, u64, String, String)> = events
        .iter()
        .filter_map(|event| match event {
            ReplaceEvent::Preview {
                rel,
                line,
                before,
                after,
                ..
            } => Some((rel.clone(), *line, before.clone(), after.clone())),
            _ => None,
        })
        .collect();
    previews.sort();
    assert_eq!(
        previews[0],
        ("a.txt".into(), 1, "Hallo Welt".into(), "Hallo Erde".into())
    );
    assert_eq!(previews.len(), 3);
    assert_eq!(read(&fixture, "a.txt"), "Hallo Welt\nWelt!\n");
    assert!(fixture.journal.history().unwrap().is_empty());
    let (summary, _) = run(
        &fixture,
        &fixture.journal,
        &request("Welt", "Erde", ReplaceSelection::All),
        true,
    );
    assert!(summary.canceled);
    assert_eq!((summary.files_changed, summary.undo_id), (0, None));
    assert_eq!(read(&fixture, "a.txt"), "Hallo Welt\nWelt!\n");
    // The journal's folder is a file: no backup, no change.
    let blocked = fixture._dir.path().join("blocked");
    std::fs::write(&blocked, b"no folder").unwrap();
    let journal = ReplaceJournal::open_at(&blocked, [7u8; 32]);
    let (summary, events) = run(
        &fixture,
        &journal,
        &request("Welt", "Erde", ReplaceSelection::All),
        false,
    );
    assert_eq!((summary.files_changed, summary.undo_id), (0, None));
    assert_eq!(read(&fixture, "a.txt"), "Hallo Welt\nWelt!\n");
    let reasons = skips(&events);
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons
        .iter()
        .all(|(_, reason)| matches!(reason, ReplaceSkip::BackupFailed(_))));
}

/// An invalid pattern fails before anything is read; a regex replacement
/// with groups works through the backend as in memory.
#[test]
fn replace_task_regex_through_the_backend() {
    let fixture = fixture();
    let mut regex = request(r"(\w+) (Welt)", "$2 $1", ReplaceSelection::All);
    regex.query.mode = TextMode::Regex;
    let (summary, _) = run(&fixture, &fixture.journal, &regex, false);
    assert_eq!(summary.files_changed, 2);
    assert_eq!(read(&fixture, "a.txt"), "Welt Hallo\nWelt!\n");
    assert_eq!(read(&fixture, "sub/c.txt"), "noch Welt eine\n");
    let mut invalid = request("(", "x", ReplaceSelection::All);
    invalid.query.mode = TextMode::Regex;
    let backend = LocalBackend::new(&fixture.root);
    let error = content_replace_with(
        &fixture.journal,
        &backend,
        &fixture.root,
        &invalid,
        &fixture.root,
        &|_| true,
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
