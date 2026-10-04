// Effective job filters, bandwidth and concurrent transfer limits in complete runs.
#[test]
fn sync_reliability_task_options_job_editor_filters_bandwidth_and_transfer_limit() {
    let pair = Pair::new();
    let days = 86_400_000;
    let specs = [("kept.bin", 2_048, TIME - 2 * days), ("kept-2.bin", 2_048, TIME - 2 * days),
        ("kept-3.bin", 2_048, TIME - 2 * days), ("small.bin", 512, TIME - 2 * days),
        ("large.bin", 4_096, TIME - 2 * days), ("new.bin", 2_048, TIME),
        ("old.bin", 2_048, TIME - 10 * days), ("ignored.bin", 2_048, TIME - 2 * days),
        ("hidden.bin", 2_048, TIME - 2 * days)];
    for (name, size, time) in specs { pair.put(PairSide::A, name, &vec![b'O'; size], time); }
    let all = empty_globset();
    let mut original = crate::syncjobs::SyncJob::new("effective options".into(), pair.roots[0].clone(), pair.roots[1].clone());
    original.direction = Direction::AtoB;
    original.compare = CompareMode::Checksum;
    original.max_transfers = 1;
    original.versions_location = VersionsLocation::AppData;
    let settings = RunSettings::for_job(&original.id);
    let first = pair.run_settings(original.checked_opts(false).unwrap(), &WalkFilter::basic(true, &all), settings.clone());
    clean(&first);
    let mut editor = crate::syncjobs::JobEditor::from_job(&original);
    editor.direction = Direction::AtoB;
    editor.compare = CompareMode::Checksum;
    editor.include_hidden = false;
    editor.ignore = "ignored.bin".into();
    editor.filter_min_size_kb = "1".into();
    editor.filter_max_size_kb = "3".into();
    editor.filter_min_age_days = "1".into();
    editor.filter_max_age_days = "5".into();
    editor.max_transfers = "1".into();
    editor.bwlimit_kbps = "1".into();
    editor.verify = true;
    editor.versions_location = VersionsLocation::AppData;
    let job = editor.build_sync_job(Some(&original)).unwrap();
    assert_eq!(job.id, original.id);
    let opts = job.checked_opts(false).unwrap();
    let globs = job.glob_set();
    let mut filter = WalkFilter::basic(job.include_hidden, &globs);
    (filter.min_size, filter.max_size, filter.after_mtime_ms, filter.before_mtime_ms) = job.checked_filter_bounds(TIME / 1_000).unwrap();
    for (name, size, time) in specs { pair.put(PairSide::A, name, &vec![b'N'; size], time + 1_000); }
    let preview = pair.preview_settings(opts, &filter, settings.clone());
    assert!(preview.error.is_none());
    assert_eq!(preview.actions.len(), 3);
    assert!(preview.actions.iter().all(|action| matches!(action, Action::CopyAtoB(name) if name.starts_with("kept"))));
    pair.b.peak.store(0, Ordering::SeqCst);
    let start = Instant::now();
    let out = pair.run_settings(opts, &filter, settings.clone());
    clean(&out);
    assert_eq!(out.state, first.state);
    assert!(start.elapsed() >= Duration::from_millis(2_400), "job bandwidth limit must delay transfers");
    assert_eq!(pair.b.peak.load(Ordering::SeqCst), 1);
    assert_eq!(pair.b.active.load(Ordering::SeqCst), 0);
    assert_eq!(out.stats.a_to_b, 3);
    for (name, size, _) in specs {
        assert_eq!(pair.bytes(PairSide::B, name), vec![if name.starts_with("kept") { b'N' } else { b'O' }; size]);
        if !name.starts_with("kept") {
            assert!(out.omissions.protects(name));
            assert_eq!(out.baseline[name], first.baseline[name]);
        }
    }
    assert_eq!(pair.stored(&out), out.baseline);
    let versions = pair.versions(&out);
    assert_eq!(versions.len(), 3);
    assert!(versions.iter().all(|entry| pair.version_bytes(entry) == vec![b'O'; 2_048]));
    assert!(versions.iter().all(|entry| entry.job_id.as_deref() == Some(job.id.as_str())));
    let noop = pair.run_settings(opts, &filter, settings);
    clean(&noop);
    assert_eq!(noop.state, out.state);
    assert_eq!(noop.stats.a_to_b + noop.stats.b_to_a + noop.stats.deleted, 0);
    assert_eq!(pair.stored(&noop), noop.baseline);
}
