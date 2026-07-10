use super::*;

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "se-persistence-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn sig() -> Sig {
    Sig {
        size: 7,
        mtime_ms: -4,
        hash: 9,
    }
}

#[test]
fn pair_identity_is_ordered_and_length_delimited() {
    assert_ne!(pair_id("a", "b"), pair_id("b", "a"));
    assert_ne!(pair_id("a|", "b"), pair_id("a", "|b"));
}

#[test]
fn pair_identity_includes_each_backend_endpoint() {
    let left_one = crate::vfs::LocalBackend::new("/left-one");
    let left_two = crate::vfs::LocalBackend::new("/left-two");
    let right = crate::vfs::LocalBackend::new("/right");
    assert_ne!(
        pair_id_for(&left_one, "/same", &right, "/same"),
        pair_id_for(&left_two, "/same", &right, "/same")
    );
}

#[test]
fn binary_baseline_round_trips_tabs_and_newlines() {
    let path = temp("roundtrip");
    let mut baseline = Baseline::new();
    baseline.insert("tab\tand\nnewline".into(), (Some(sig()), None));
    save_baseline(&path, &baseline).unwrap();
    assert_eq!(load_baseline(&path).unwrap(), baseline);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn duplicate_binary_path_is_rejected() {
    let mut bytes = BASELINE_MAGIC.to_vec();
    for _ in 0..2 {
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.push(b'x');
        write_sig(&mut bytes, Some(sig())).unwrap();
        write_sig(&mut bytes, None).unwrap();
    }
    assert!(parse_binary(&bytes[BASELINE_MAGIC.len()..]).is_err());
}

#[test]
fn huge_retention_days_saturate_without_pruning() {
    let root = temp("retention");
    std::fs::create_dir_all(root.join("1")).unwrap();
    prune_versions(
        &root,
        &Versioning {
            scheme: VersioningScheme::Days,
            days: u64::MAX,
            count: 0,
        },
    )
    .unwrap();
    assert!(root.join("1").exists());
    std::fs::remove_dir_all(root).unwrap();
}
