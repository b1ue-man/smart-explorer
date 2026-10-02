use super::*;

fn unique_hex() -> String {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("random id");
    format!("{:016x}", u64::from_be_bytes(bytes))
}

fn sig(size: u64) -> Sig {
    Sig {
        size,
        mtime_ms: 1_700_000_000_000,
        hash: 0,
    }
}

fn job_key(pair: &str, lock: &str, job: &str) -> StateKey {
    StateKey {
        pair_id: pair.to_string(),
        lock_id: lock.to_string(),
        owner: StateOwner::Job(job.to_string()),
        replica_a: ReplicaRef::Marker("replica-a".into()),
        replica_b: ReplicaRef::Volume("volume-b".into()),
    }
}

#[test]
fn review_task_state_files_are_per_owner_and_replica_pair() {
    let pair = unique_hex();
    let legacy = StateKey::legacy(&pair, &pair);
    assert_eq!(baseline_file(&legacy).unwrap(), baseline_path(&pair));

    let first = baseline_file(&job_key(&pair, &pair, "job-1")).unwrap();
    let second = baseline_file(&job_key(&pair, &pair, "job-2")).unwrap();
    assert_ne!(
        first, second,
        "a new job never inherits another job's state"
    );
    let mut rotated = job_key(&pair, &pair, "job-1");
    rotated.replica_b = ReplicaRef::Marker("other drive".into());
    assert_ne!(first, baseline_file(&rotated).unwrap());
    assert!(first.ends_with(format!(
        "{PAIRS_DIR}/{pair}/job-job-1.{}.{BASELINE_EXTENSION}",
        replica_token(
            &ReplicaRef::Marker("replica-a".into()),
            &ReplicaRef::Volume("volume-b".into())
        )
    )));

    assert!(baseline_file(&job_key(&pair, &pair, "../escape")).is_err());
    assert!(baseline_file(&job_key("not/hex", &pair, "job-1")).is_err());
}

#[test]
fn review_task_merge_and_forget_keep_other_owners() {
    let pair = unique_hex();
    let lock = PairLock::acquire(&pair).expect("pair lock");
    let mine = job_key(&pair, &pair, "merge-mine");
    let other = job_key(&pair, &pair, "merge-other");
    merge_baseline_entries(
        &lock,
        &mine,
        &[
            ("keep.txt".into(), (Some(sig(1)), Some(sig(1)))),
            ("gone.txt".into(), (Some(sig(2)), Some(sig(2)))),
        ],
    )
    .unwrap();
    merge_baseline_entries(&lock, &mine, &[("gone.txt".into(), (None, None))]).unwrap();
    merge_baseline_entries(&lock, &other, &[("x".into(), (Some(sig(3)), None))]).unwrap();
    let stored = load_baseline(&baseline_file(&mine).unwrap()).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored["keep.txt"], (Some(sig(1)), Some(sig(1))));

    let foreign = PairLock::acquire(&unique_hex()).expect("other lock");
    assert_eq!(
        merge_baseline_entries(&foreign, &mine, &[])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );

    forget_job_state("merge-mine").unwrap();
    assert!(!baseline_file(&mine).unwrap().exists());
    assert!(baseline_file(&other).unwrap().exists());
    forget_pair_state(&pair).unwrap();
    assert!(!baseline_file(&other).unwrap().exists());
    assert!(!pair_dir(&pair).exists());
}
