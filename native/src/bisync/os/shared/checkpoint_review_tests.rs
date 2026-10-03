use super::*;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::AtomicBool;
use crate::vfs::Backend;

pub(super) struct Fixture {
    pub(super) root: std::path::PathBuf,
    pub(super) key: StateKey,
    pub(super) backend: crate::vfs::LocalBackend,
    pub(super) a: String,
    pub(super) b: String,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).unwrap();
        let id = format!("{:016x}", u64::from_be_bytes(random));
        let root = std::env::temp_dir().join(format!("se-eplan-{id}"));
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let a = root.join("a").to_str().unwrap().to_string();
        let b = root.join("b").to_str().unwrap().to_string();
        let backend = crate::vfs::LocalBackend::new("/");
        let key = StateKey::legacy(&pair_id_for(&backend, &a, &backend, &b),
            &pair_lock_id(&backend, &a, &backend, &b));
        Self { root, key, backend, a, b }
    }

    pub(super) fn endpoints(&self) -> incremental::SyncEndpoints<'_> {
        incremental::SyncEndpoints::new(&self.backend, &self.a, &self.backend, &self.b)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(replica_state::pair_dir(&self.key.pair_id));
        let _ = std::fs::remove_file(baseline_path(&self.key.pair_id));
        let _ = std::fs::remove_file(baseline_path(&self.key.pair_id).with_extension("journal"));
        let _ = std::fs::remove_file(baseline_path(&self.key.pair_id).with_extension("dirs.json"));
        let _ = std::fs::remove_dir_all(versions_dir(&self.key.pair_id));
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn sig(size: u64, time: i64) -> Sig { Sig { size, mtime_ms: time, hash: 9 } }

#[test]
fn review_task_checkpoint_journal_recovers_and_truncates_only_a_torn_tail() {
    let fixture = Fixture::new();
    let keys = KeyPolicy { fold_case: true };
    save_baseline(&baseline_file(&fixture.key).unwrap(), &Baseline::from([
        ("Old.TXT".into(), (Some(sig(1, 1)), Some(sig(1, 2)))),
        ("protected/x".into(), (Some(sig(2, 3)), Some(sig(2, 4)))),
    ])).unwrap();
    let (mut journal, _, _) = checkpoint_journal::Journal::load(&fixture.key, keys).unwrap();
    let frame = checkpoint_journal::Frame { fold_case: true,
        records: vec![("old.txt".into(), (Some(sig(3, 5)), Some(sig(3, 6))))],
        ..Default::default() };
    journal.append(&frame).unwrap();
    let path = baseline_file(&fixture.key).unwrap().with_extension("journal");
    let mut tail = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    tail.write_all(&12u64.to_be_bytes()).unwrap();
    tail.write_all(&[0u8; 35]).unwrap();
    tail.sync_all().unwrap();
    drop(tail);
    let (mut recovered, records, _) = checkpoint_journal::Journal::load(&fixture.key, keys).unwrap();
    assert!(!records.baseline.contains_key("Old.TXT"));
    assert_eq!(records.baseline["old.txt"], (Some(sig(3, 5)), Some(sig(3, 6))));
    assert!(records.baseline.contains_key("protected/x"));
    recovered.append(&checkpoint_journal::Frame { fold_case: true,
        records: vec![("next".into(), (Some(sig(4, 7)), None))], ..Default::default() }).unwrap();
    let (_, records, _) = checkpoint_journal::Journal::load(&fixture.key, keys).unwrap();
    assert_eq!(records.baseline.len(), 3);
    assert!(records.baseline.contains_key("next"));
}

#[test]
fn review_task_checkpoint_rejects_a_complete_corrupt_frame() {
    let fixture = Fixture::new();
    let (mut journal, _, _) = checkpoint_journal::Journal::load(&fixture.key, KeyPolicy::default()).unwrap();
    journal.append(&checkpoint_journal::Frame { records: vec![("x".into(), (Some(sig(1, 1)), None))], ..Default::default() }).unwrap();
    let path = baseline_file(&fixture.key).unwrap().with_extension("journal");
    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(8)).unwrap();
    let mut first = [0u8; 1];
    file.read_exact(&mut first).unwrap();
    first[0] ^= 1;
    file.seek(SeekFrom::Start(8)).unwrap();
    file.write_all(&first).unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert!(checkpoint_journal::Journal::load(&fixture.key, KeyPolicy::default()).is_err());
}

#[test]
fn review_task_checkpoint_keeps_successes_and_old_deferred_entries_on_stop() {
    let fixture = Fixture::new();
    save_baseline(&baseline_file(&fixture.key).unwrap(), &Baseline::from([
        ("bad".into(), (Some(sig(2, 3)), Some(sig(2, 4))))])).unwrap();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = checkpoint_run::CheckpointSink::new(fixture.endpoints(), &lock, &fixture.key,
        KeyPolicy::default(), None).unwrap();
    sink.completed(CompletedAction { rel: "good".into(), kind: CompletedKind::Copied { from: PairSide::A },
        src_sig: Some(sig(7, 100)), dst_sig: Some(sig(7, 999)), durable: true });
    sink.deferred("bad", "changed during copy");
    sink.stopped(RunStop::TargetFull { side: PairSide::B });
    assert!(sink.should_stop());
    let finished = sink.finish();
    assert!(finished.error.is_none());
    assert_eq!(finished.baseline["bad"], (Some(sig(2, 3)), Some(sig(2, 4))));
    assert_eq!(finished.baseline["good"], (Some(sig(7, 100)), Some(sig(7, 999))));
    assert_eq!(load_baseline(&baseline_file(&fixture.key).unwrap()).unwrap(), finished.baseline);
    assert_eq!(finished.deferred.len(), 1);
}

#[test]
fn review_task_checkpoint_flushes_before_the_end_of_a_long_run() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = checkpoint_run::CheckpointSink::new(fixture.endpoints(), &lock, &fixture.key,
        KeyPolicy::default(), None).unwrap();
    for n in 0..64 {
        sink.completed(CompletedAction { rel: format!("file-{n}"), kind: CompletedKind::Copied { from: PairSide::A },
            src_sig: Some(sig(1, n)), dst_sig: Some(sig(1, n + 100)), durable: true });
    }
    let (_, stored, _) = checkpoint_journal::Journal::load(&fixture.key, KeyPolicy::default()).unwrap();
    assert_eq!(stored.baseline.len(), 64, "completed batch is durable before finish");
    assert_eq!(sink.finish().baseline.len(), 64);
}

#[test]
fn review_task_checkpoint_timer_saves_while_the_next_transfer_is_slow() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let sink = checkpoint_run::CheckpointSink::new(fixture.endpoints(), &lock, &fixture.key,
        KeyPolicy::default(), None).unwrap();
    sink.completed(CompletedAction { rel: "completed".into(), kind: CompletedKind::Copied { from: PairSide::A },
        src_sig: Some(sig(1, 2)), dst_sig: Some(sig(1, 3)), durable: true });
    sink.during(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let (_, stored, _) = checkpoint_journal::Journal::load(&fixture.key, KeyPolicy::default()).unwrap();
        assert!(stored.baseline.contains_key("completed"));
    });
    assert!(sink.finish().error.is_none());
}

#[test]
fn review_task_external_merge_replays_checkpoints_before_updating_one_entry() {
    let fixture = Fixture::new();
    let lock = PairLock::acquire(&fixture.key.lock_id).unwrap();
    let (mut journal, _, _) = checkpoint_journal::Journal::load(&fixture.key, KeyPolicy::default()).unwrap();
    journal.append(&checkpoint_journal::Frame { records: vec![("completed".into(), (Some(sig(1, 2)), Some(sig(1, 3))))],
        ..Default::default() }).unwrap();
    merge_baseline_entries(&lock, &fixture.key, &[("resolved".into(), (Some(sig(2, 4)), Some(sig(2, 5))))]).unwrap();
    let stored = load_baseline(&baseline_file(&fixture.key).unwrap()).unwrap();
    assert!(stored.contains_key("completed") && stored.contains_key("resolved"));
}

#[test]
fn review_task_checkpoint_budget_rejects_delta_before_journaling_it() {
    let records = baseline_records::RecordBook::new(Baseline::from([
        ("old".into(), (Some(sig(1, 1)), None))]), KeyPolicy::default());
    let limits = SyncLimits { walk_entries: 1, walk_text_bytes: 10, state_entries: 1, state_text_bytes: 10 };
    let add = checkpoint_journal::Frame { records: vec![("new".into(), (Some(sig(1, 2)), None))], ..Default::default() };
    assert!(add.ensure_fits(&records, &DirSet::new(), 0, limits).is_err());
    let replace = checkpoint_journal::Frame { forget: vec!["old".into()], ..add };
    assert!(replace.ensure_fits(&records, &DirSet::new(), 0, limits).is_ok());
    assert_eq!(records.baseline.len(), 1);
}

#[test]
fn review_task_corrupt_optional_index_does_not_block_completed_file_work() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("a/file"), b"before").unwrap();
    let ignore = empty_globset();
    let filter = WalkFilter::basic(true, &ignore);
    let cancel = AtomicBool::new(false);
    let opts = BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror, ..Default::default() };
    let store = fixture.root.join("index.sqlite");
    let first = orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert_eq!(first.stats.errors, 0, "{:?}", first.errors);
    assert_eq!(first.stats.a_to_b, 1);
    std::fs::write(&store, b"broken sqlite cache").unwrap();
    std::fs::write(fixture.root.join("a/file"), b"after, longer").unwrap();
    let next = orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert_eq!(next.stats.errors, 0, "{:?}", next.errors);
    assert_eq!(next.stats.a_to_b, 1);
    assert_eq!(std::fs::read(fixture.root.join("b/file")).unwrap(), b"after, longer");
}

#[test]
fn review_task_daily_target_verification_finds_untracked_mirror_orphans() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join("a/file"), b"kept").unwrap();
    let ignore = empty_globset();
    let filter = WalkFilter::basic(true, &ignore);
    let cancel = AtomicBool::new(false);
    let opts = BisyncOptions { direction: Direction::AtoB, delete: DeletePolicy::Mirror,
        verify_target_secs: 86_400, ..Default::default() };
    let store = fixture.root.join("index.sqlite");
    let first = orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert_eq!(first.stats.errors, 0, "{:?}", first.errors);
    std::fs::write(fixture.root.join("b/orphan"), b"orphan").unwrap();
    let quick = orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert_eq!(quick.stats.errors, 0, "{:?}", quick.errors);
    assert!(fixture.root.join("b/orphan").exists());
    let key = first.state.unwrap();
    let mut history = state_metadata::load_history(&key).unwrap().unwrap();
    history.full_ms = 0;
    state_metadata::save_history(&key, &history).unwrap();
    let daily = orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
    assert_eq!(daily.stats.errors, 0, "{:?}", daily.errors);
    assert_eq!(daily.stats.a_to_b, 0);
    assert!(!fixture.root.join("b/orphan").exists());
}
