//! Plan rules at the two engine hooks (vault plan C5.4 R4, OE4). Without
//! rules, and with the default rules, planning and apply of every existing
//! job kind stay unchanged; installed rules adjust the plan and run apply in
//! phases. Fake rules on plain files, no vault.
use super::super::apply::ApplyReport;
use super::super::checkpoint::{ApplyScope, Collected, CollectingSink};
use super::super::completion::{ApplySink, CompletedAction};
use super::super::incremental::SyncEndpoints;
use super::super::keys::{PathAliases, Spellings};
use super::super::orchestration_plan::prepare_spelled;
use super::super::plan_rules::{
    self, phase_plan, DefaultRules, PairPlan, PhasePlan, PlanRules, DEFERRED_BY_RULE, HELD_BACK,
};
use super::super::plan_types::PlanContext;
use super::super::run_types::StateOwner;
use super::super::test_remote::{FakeRemote, Hook, REMOTE_ROOT};
use super::super::versions::{RunVersions, VersionsContext};
use super::super::*;
use crate::vfs::LocalBackend;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Every file below `root` as (relative path, bytes), sorted.
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn visit(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, out);
            } else {
                let rel = forward(path.strip_prefix(root).unwrap());
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

fn rel_of(action: &Action) -> &str {
    super::super::core::action_rel(action)
}

fn is_node(rel: &str) -> bool {
    rel.ends_with("/dir.c9r") || rel.ends_with("/symlink.c9r")
}

fn is_delete(action: &Action) -> bool {
    matches!(action, Action::DeleteA(_) | Action::DeleteB(_))
}

/// Fake rules shaped like R4: phase 1 creates and changes everything that
/// is no node file and deletes node files; phase 2 does the rest. After an
/// incomplete earlier action every node creation waits.
struct NodesAfterContent;

impl PlanRules for NodesAfterContent {
    fn phases(&self, actions: &[Action]) -> Vec<Vec<usize>> {
        let (mut first, mut second) = (Vec::new(), Vec::new());
        for (index, action) in actions.iter().enumerate() {
            if is_node(rel_of(action)) == is_delete(action) {
                first.push(index);
            } else {
                second.push(index);
            }
        }
        vec![first, second]
    }

    fn held_back(&self, _incomplete: &[&Action], coming: &[&Action]) -> Vec<usize> {
        coming
            .iter()
            .enumerate()
            .filter(|(_, action)| is_node(rel_of(action)) && !is_delete(action))
            .map(|(position, _)| position)
            .collect()
    }
}

/// Fake rules with a rule stage that drops every planned action.
struct DropAll;

impl PlanRules for DropAll {
    fn adjust(&self, plan: &mut PairPlan) {
        plan.actions.clear();
    }
}

/// Fake rules that name only the first action and leave out a bad index.
struct OnlyFirst;

impl PlanRules for OnlyFirst {
    fn phases(&self, _actions: &[Action]) -> Vec<Vec<usize>> {
        vec![vec![0, 0, 99], vec![]]
    }
}

fn every_kind() -> Vec<Action> {
    vec![
        Action::CopyAtoB("a.txt".into()),
        Action::CopyBtoA("b.txt".into()),
        Action::DeleteA("c.txt".into()),
        Action::DeleteB("d.txt".into()),
        Action::KeepBothAtoB("e.txt".into()),
        Action::KeepBothBtoA("f.txt".into()),
        Action::FinalizeMoveAtoB("g.txt".into()),
        Action::FinalizeMoveBtoA("h.txt".into()),
    ]
}

fn sig(size: u64) -> Sig {
    Sig {
        size,
        mtime_ms: 1_000,
        hash: 0,
    }
}

fn side(entries: &[(&str, u64)]) -> SideSnapshot {
    let mut side = SideSnapshot::new(false);
    for (rel, size) in entries {
        side.tree.insert(rel.to_string(), sig(*size));
    }
    side
}

/// Plans one fixed pair with `rules` installed (or none); returns the plan
/// and both sides' omissions as text.
fn plan_with(rules: Option<Arc<dyn PlanRules>>) -> (PairPlan, String) {
    let dir = tempfile::tempdir().unwrap();
    let (root_a, root_b) = (
        forward(&dir.path().join("a")),
        forward(&dir.path().join("b")),
    );
    let (backend_a, backend_b) = (LocalBackend::new(&root_a), LocalBackend::new(&root_b));
    let endpoints = SyncEndpoints::new(&backend_a, &root_a, &backend_b, &root_b);
    let mut a = side(&[("a.txt", 1), ("both.txt", 2), ("dir/x.txt", 5)]);
    let mut b = side(&[("b.txt", 3), ("both.txt", 4)]);
    // A file both sides had at the last run and both lost since (forget).
    let mut base = Baseline::new();
    base.insert("gone.txt".into(), (Some(sig(9)), Some(sig(9))));
    let ctx = PlanContext::new(BisyncOptions::default());
    let _guard = rules.map(plan_rules::enter);
    let plan = prepare_spelled(
        endpoints,
        &mut a,
        &mut b,
        &base,
        &ctx,
        &PathAliases::default(),
        &AtomicBool::new(false),
    );
    (plan, format!("{:?} {:?}", a.omissions, b.omissions))
}

/// One fixed local apply (copies both ways, deletions, a keep-both) with
/// `rules` installed (or none): stats, errors and both sides' files.
fn apply_with(rules: Option<Arc<dyn PlanRules>>) -> (String, Vec<(String, String)>, String) {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for (rel, content) in [("one.txt", "1"), ("sub/two.txt", "22"), ("both.txt", "A")] {
        write(a.path(), rel, content.as_bytes());
    }
    for (rel, content) in [("three.txt", "333"), ("old.txt", "x"), ("both.txt", "B")] {
        write(b.path(), rel, content.as_bytes());
    }
    let actions = vec![
        Action::CopyAtoB("one.txt".into()),
        Action::CopyAtoB("sub/two.txt".into()),
        Action::CopyBtoA("three.txt".into()),
        Action::DeleteB("old.txt".into()),
        Action::KeepBothAtoB("both.txt".into()),
    ];
    let (root_a, root_b) = (forward(a.path()), forward(b.path()));
    let (backend_a, backend_b) = (LocalBackend::new(&root_a), LocalBackend::new(&root_b));
    let _guard = rules.map(plan_rules::enter);
    let mut errors = Vec::new();
    let stats = apply(
        &actions,
        &backend_a,
        &root_a,
        &backend_b,
        &root_b,
        BisyncOptions::default(),
        versions.path(),
        &mut errors,
        &AtomicBool::new(false),
    );
    let keep_both_name = |rel: String| {
        if rel.contains("Konflikt") {
            "Konfliktkopie".to_string()
        } else {
            rel
        }
    };
    let state = [a.path(), b.path()]
        .iter()
        .map(|root| {
            files(root)
                .into_iter()
                .map(|(rel, bytes)| (keep_both_name(rel), bytes))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    (format!("{stats:?}"), errors, format!("{state:?}"))
}

#[test]
fn vault_task_plan_rules_default_single_phase() {
    // No rules are installed for the existing job kinds.
    assert!(plan_rules::current().is_none());

    // Apply: the default is one phase of all actions in plan order, which
    // the engine runs through the unchanged pool call; this holds for every
    // action kind and for empty and single-action plans.
    let actions = every_kind();
    for count in 0..=actions.len() {
        assert_eq!(
            phase_plan(&DefaultRules, &actions[..count]),
            None,
            "{count}"
        );
    }
    assert_eq!(
        DefaultRules.phases(&actions),
        vec![(0..8).collect::<Vec<_>>()]
    );
    assert!(DefaultRules
        .held_back(&[&actions[0]], &[&actions[1]])
        .is_empty());

    // Normalization of other rules: repeated and out-of-range indices are
    // ignored, empty phases dropped, unnamed actions deferred.
    assert_eq!(
        phase_plan(&OnlyFirst, &actions),
        Some(PhasePlan {
            phases: vec![vec![0]],
            deferred: (1..8).collect(),
        })
    );
    assert_eq!(OnlyFirst.deferral_reason(&actions[1]), DEFERRED_BY_RULE);
    assert_eq!(phase_plan(&OnlyFirst, &actions[..1]), None);
    assert!(phase_plan(&NodesAfterContent, &actions).is_some());

    // The guard restores the previous rules, also when nested.
    {
        let _outer = plan_rules::enter(Arc::new(DefaultRules));
        {
            let _inner = plan_rules::enter(Arc::new(DropAll));
            assert!(plan_rules::current().is_some());
        }
        assert!(plan_rules::current().is_some());
    }
    assert!(plan_rules::current().is_none());

    // Planning (full scan and preview share `prepare_spelled`): the default
    // rule stage leaves plan and omissions exactly as without rules, while
    // installed rules do reach the plan.
    let (plain, plain_omissions) = plan_with(None);
    let (default, default_omissions) = plan_with(Some(Arc::new(DefaultRules)));
    assert!(!plain.actions.is_empty());
    assert_eq!(format!("{plain:?}"), format!("{default:?}"));
    assert_eq!(plain_omissions, default_omissions);
    let (dropped, _) = plan_with(Some(Arc::new(DropAll)));
    assert!(dropped.actions.is_empty());
    assert_eq!(
        format!("{:?}", dropped.conflicts),
        format!("{:?}", plain.conflicts)
    );

    // Apply on identical fixtures: the same stats, errors and files with
    // and without the default rules.
    let plain = apply_with(None);
    let default = apply_with(Some(Arc::new(DefaultRules)));
    assert!(plain.1.is_empty(), "{:?}", plain.1);
    assert!(plain.2.contains("three.txt") && plain.2.contains("sub/two.txt"));
    assert_eq!(plain, default);
}

/// Records apply events and finished actions in one ordered log.
struct Recorder {
    log: Arc<Mutex<Vec<String>>>,
    inner: CollectingSink,
}

impl ApplySink for Recorder {
    fn completed(&self, action: CompletedAction) {
        let line = format!("done:{}", action.rel);
        self.log.lock().unwrap().push(line);
        self.inner.completed(action);
    }

    fn deferred(&self, rel: &str, reason: &str) {
        self.inner.deferred(rel, reason);
    }
}

/// A hook that logs every read and stage write and slows down the reads
/// below `slow` (the phase-1 work of its side).
fn logging_hook(log: &Arc<Mutex<Vec<String>>>, slow: &'static str) -> Hook {
    let log = log.clone();
    Arc::new(move |operation: &str, path: &str| {
        if operation == "read" || operation == "write" {
            log.lock().unwrap().push(format!("{operation}:{path}"));
        }
        if operation == "read" && path.contains(slow) {
            std::thread::sleep(Duration::from_millis(80));
        }
    })
}

/// 1 = phase-1 work, 2 = phase-2 work, by the folders of the fixture.
fn phase_of(event: &str) -> Option<u8> {
    let counted = ["read:", "write:", "done:"]
        .iter()
        .any(|prefix| event.starts_with(prefix));
    if !counted {
        return None;
    }
    if event.contains("d/AA/") || event.contains("d/EE/") {
        Some(1)
    } else if event.contains("d/CC/") || event.contains("d/GG/") {
        Some(2)
    } else {
        None
    }
}

/// Runs the R4-shaped fake rules over a fixture: content and node files on
/// A to copy, a node file and content entries on B to delete. Phase-2
/// actions come first in plan order; phase-1 reads are slow.
fn run_phased(fail_writes_to: Option<&str>) -> (Vec<String>, Collected, ApplyReport) {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let versions = tempfile::tempdir().unwrap();
    for rel in [
        "d/AA/BBBB/dirid.c9r",
        "d/AA/BBBB/f1.c9r",
        "d/AA/BBBB/n1.c9s/contents.c9r",
        "d/CC/DDDD/x.c9r/dir.c9r",
        "d/CC/DDDD/s.c9r/symlink.c9r",
    ] {
        write(a.path(), rel, rel.as_bytes());
    }
    for rel in [
        "d/EE/FFFF/y.c9r/dir.c9r",
        "d/GG/HHHH/dirid.c9r",
        "d/GG/HHHH/g.c9r",
    ] {
        write(b.path(), rel, rel.as_bytes());
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let side_a = FakeRemote::new(a.path(), "phases-a").with_hook(logging_hook(&log, "/d/AA/"));
    let mut side_b = FakeRemote::new(b.path(), "phases-b").with_hook(logging_hook(&log, "/d/EE/"));
    if let Some(name) = fail_writes_to {
        side_b = side_b.failing_writes_to(name);
    }
    let cancel = AtomicBool::new(false);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let tree_a = walk_files(&side_a, REMOTE_ROOT, &cancel, &filter, HashMode::None, None).unwrap();
    let tree_b = walk_files(&side_b, REMOTE_ROOT, &cancel, &filter, HashMode::None, None).unwrap();
    log.lock().unwrap().clear();
    let actions: Vec<Action> = [
        "+d/CC/DDDD/x.c9r/dir.c9r",
        "+d/CC/DDDD/s.c9r/symlink.c9r",
        "-d/GG/HHHH/dirid.c9r",
        "-d/GG/HHHH/g.c9r",
        "+d/AA/BBBB/dirid.c9r",
        "+d/AA/BBBB/f1.c9r",
        "+d/AA/BBBB/n1.c9s/contents.c9r",
        "-d/EE/FFFF/y.c9r/dir.c9r",
    ]
    .iter()
    .map(|entry| match entry.split_at(1) {
        ("+", rel) => Action::CopyAtoB(rel.to_string()),
        (_, rel) => Action::DeleteB(rel.to_string()),
    })
    .collect();
    let opts = BisyncOptions::default();
    let pair = pair_id_for(&side_a, REMOTE_ROOT, &side_b, REMOTE_ROOT);
    let context = VersionsContext::new(
        &pair,
        StateOwner::AdHoc,
        VersionsLocation::AppData,
        opts.versioning,
    );
    let run_versions = RunVersions::with_app_data(context, versions.path().to_path_buf());
    let spellings = Spellings::default();
    let sink = Recorder {
        log: log.clone(),
        inner: CollectingSink::default(),
    };
    let scope = ApplyScope {
        sink: &sink,
        versions: &run_versions,
        spellings: &spellings,
    };
    let mut errors = Vec::new();
    let _rules = plan_rules::enter(Arc::new(NodesAfterContent));
    let report = super::run(
        &actions,
        &[],
        Some((&tree_a, &tree_b)),
        SyncEndpoints::new(&side_a, REMOTE_ROOT, &side_b, REMOTE_ROOT),
        opts,
        &scope,
        &mut errors,
        &cancel,
        false,
    );
    assert!(b.path().join("d/AA/BBBB/f1.c9r").exists(), "{errors:?}");
    assert!(!b.path().join("d/GG/HHHH/g.c9r").exists(), "{errors:?}");
    assert!(
        !b.path().join("d/EE/FFFF/y.c9r/dir.c9r").exists(),
        "{errors:?}"
    );
    let events = log.lock().unwrap().clone();
    (events, sink.inner.take(), report)
}

#[test]
fn vault_task_sync_phases_content_before_node() {
    // All actions succeed: no work of phase 2 starts before the last action
    // of phase 1 finished, although phase 2 comes first in plan order and
    // phase 1 is slow. Node creations follow content, deletions below `d/`
    // follow the node deletion.
    let (events, collected, report) = run_phased(None);
    let last_first = events
        .iter()
        .rposition(|event| event.starts_with("done:") && phase_of(event) == Some(1))
        .expect("phase 1 finished");
    let first_second = events
        .iter()
        .position(|event| phase_of(event) == Some(2))
        .expect("phase 2 ran");
    assert!(last_first < first_second, "{events:#?}");
    assert_eq!(report.completed.len(), 8, "{events:#?}");
    assert_eq!(report.stats.errors, 0);
    assert!(collected.deferred.is_empty(), "{:?}", collected.deferred);
    let done: Vec<&str> = events
        .iter()
        .filter_map(|event| event.strip_prefix("done:"))
        .collect();
    assert_eq!(done.len(), 8, "{done:?}");

    // A phase-1 action fails: the node creations of phase 2 stay planned
    // for the next run (deferred, not started); the other work runs.
    let (events, collected, report) = run_phased(Some("dirid"));
    let held: Vec<&str> = collected
        .deferred
        .iter()
        .filter(|(_, reason)| reason == HELD_BACK)
        .map(|(rel, _)| rel.as_str())
        .collect();
    assert_eq!(
        held,
        ["d/CC/DDDD/x.c9r/dir.c9r", "d/CC/DDDD/s.c9r/symlink.c9r"],
        "{:?}",
        collected.deferred
    );
    assert!(
        !events.iter().any(|event| event.contains("d/CC/")),
        "{events:#?}"
    );
    for rel in [
        "d/AA/BBBB/dirid.c9r",
        "d/CC/DDDD/x.c9r/dir.c9r",
        "d/CC/DDDD/s.c9r/symlink.c9r",
    ] {
        assert!(
            !report.completed.iter().any(|action| rel_of(action) == rel),
            "{rel}"
        );
    }
    assert!(report
        .completed
        .contains(&Action::DeleteB("d/GG/HHHH/g.c9r".to_string())));
}
