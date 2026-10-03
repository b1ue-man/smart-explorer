//! Acceptance cases collected by the final remote task suite; never run locally.
use super::*;
use super::super::test_remote::{FakeRemote, REMOTE_ROOT};
use crate::vfs::LocalBackend;
use super::super::merge_task_fixture::*;

#[test]
fn review_task_merge_rejects_changed_bytes_even_with_same_size_and_time() {
    let ad = tempfile::tempdir().unwrap(); let bd = tempfile::tempdir().unwrap();
    let ra = forward(ad.path()); let rb = forward(bd.path());
    let a = LocalBackend::new(&ra); let b = LocalBackend::new(&rb);
    let pa = format!("{ra}/f.txt"); let pb = format!("{rb}/f.txt");
    std::fs::write(&pa, b"alpha").unwrap(); std::fs::write(&pb, b"bravo").unwrap();
    let conflict = Conflict { rel: "f.txt".into(), a: Some(signature(&a,&pa)), b: Some(signature(&b,&pb)), duplicates: None };
    let key = state(&a,&ra,&b,&rb); seed(&key,&conflict);
    std::fs::write(&pa, b"omega").unwrap();
    crate::vfs::finish_stage(&a, &pa, crate::vfs::StageFinish {
        mtime_ms: conflict.a.map(|sig| sig.mtime_ms), ..Default::default()
    }).unwrap();
    let result = merge_recorded_for_key(&a,&ra,&b,&rb,&key,&conflict,
        OriginalContent { signature: conflict.a, bytes: Some(b"alpha") },
        OriginalContent { signature: conflict.b, bytes: Some(b"bravo") },
        MergeChoice::Write(b"joined"), &AtomicBool::new(false), |_| {});
    assert!(result.is_err());
    assert_eq!(std::fs::read(&pa).unwrap(), b"omega");
    assert_eq!(std::fs::read(&pb).unwrap(), b"bravo");
    assert_eq!(baseline(&key)["f.txt"], (conflict.a,conflict.b));
    clean(&key);
}

#[test]
fn review_task_merge_partial_publication_keeps_conflict_basis_and_retries() {
    let ad = tempfile::tempdir().unwrap(); let bd = tempfile::tempdir().unwrap();
    std::fs::write(ad.path().join("f.txt"),b"alpha").unwrap();
    std::fs::write(bd.path().join("f.txt"),b"bravo").unwrap();
    let a = FakeRemote::new(ad.path(),"merge-first");
    let b = PublishOnce { inner: FakeRemote::new(bd.path(),"merge-second"), fail: AtomicBool::new(true) };
    let pa = format!("{REMOTE_ROOT}/f.txt");
    let conflict = Conflict { rel:"f.txt".into(),a:Some(signature(&a,&pa)),b:Some(signature(&b,&pa)),duplicates:None };
    let key=state(&a,REMOTE_ROOT,&b,REMOTE_ROOT); seed(&key,&conflict);
    let oa=OriginalContent {signature:conflict.a,bytes:Some(b"alpha")};
    let ob=OriginalContent {signature:conflict.b,bytes:Some(b"bravo")};
    let first=merge_recorded_for_key(&a,REMOTE_ROOT,&b,REMOTE_ROOT,&key,&conflict,
        oa,ob,MergeChoice::Write(b"joined"),&AtomicBool::new(false),|_|{}).unwrap_err();
    assert!(first.partial.confirmed_a); assert!(!first.partial.confirmed_b);
    assert_eq!(std::fs::read(ad.path().join("f.txt")).unwrap(),b"joined");
    assert_eq!(std::fs::read(bd.path().join("f.txt")).unwrap(),b"bravo");
    assert_eq!(baseline(&key)["f.txt"],(conflict.a,conflict.b));
    let pending=super::super::merge_resume::pending_merge_for_key(&a,REMOTE_ROOT,&b,REMOTE_ROOT,&key,"f.txt")
        .unwrap().unwrap();
    assert_eq!(pending.original_a.as_deref(),Some(b"alpha".as_slice()));
    assert_eq!(pending.original_b.as_deref(),Some(b"bravo".as_slice()));
    assert_eq!(pending.merged,b"joined");
    assert_eq!(pending.confirmed_a,first.partial.a);
    assert!(pending.confirmed_b.is_none());
    {
        let lock=PairLock::acquire(&key.lock_id).unwrap();
        assert_eq!(super::super::merge_resume::pending_merge_relatives(&lock,&key).unwrap(),vec!["f.txt"]);
    }
    let blocked=super::super::resolve_conflict::resolve_recorded(&a,REMOTE_ROOT,&b,REMOTE_ROOT,
        &conflict,true,None,&key,&AtomicBool::new(false),|_|{}).unwrap_err();
    assert_eq!(blocked.kind(),io::ErrorKind::WouldBlock);
    // No in-memory draft is needed: the retry uses the private durable inputs.
    let retry=merge_recorded_for_key(&a,REMOTE_ROOT,&b,REMOTE_ROOT,&key,&conflict,
        OriginalContent {signature:pending.conflict.a,bytes:pending.original_a.as_deref()},
        OriginalContent {signature:pending.conflict.b,bytes:pending.original_b.as_deref()},
        MergeChoice::Write(&pending.merged),&AtomicBool::new(false),|_|{}).unwrap();
    assert!(retry.confirmed_a && retry.confirmed_b);
    assert_eq!(retry.a, first.partial.a, "the first durable write is retained");
    assert_eq!(std::fs::read(bd.path().join("f.txt")).unwrap(),b"joined");
    assert_eq!(retry.baseline["f.txt"],(retry.a,retry.b));
    assert_eq!(retry.a.unwrap().hash,super::super::snapshot_hash::md5_to_u64(&md5::compute(b"joined").0));
    assert!(super::super::merge_resume::pending_merge_for_key(&a,REMOTE_ROOT,&b,REMOTE_ROOT,&key,"f.txt")
        .unwrap().is_none());
    clean(&key);
}

#[test]
fn review_task_merge_keep_both_preserves_loser_on_both_sides() {
    let ad=tempfile::tempdir().unwrap(); let bd=tempfile::tempdir().unwrap();
    let ra=forward(ad.path()); let rb=forward(bd.path());
    let a=LocalBackend::new(&ra); let b=LocalBackend::new(&rb);
    let pa=format!("{ra}/f.txt"); let pb=format!("{rb}/f.txt");
    std::fs::write(&pa,b"alpha").unwrap(); std::fs::write(&pb,b"bravo").unwrap();
    let conflict=Conflict {rel:"f.txt".into(),a:Some(signature(&a,&pa)),b:Some(signature(&b,&pb)),duplicates:None};
    let key=state(&a,&ra,&b,&rb); seed(&key,&conflict);
    let report=merge_recorded_for_key(&a,&ra,&b,&rb,&key,&conflict,
        OriginalContent{signature:conflict.a,bytes:Some(b"alpha")},
        OriginalContent{signature:conflict.b,bytes:Some(b"bravo")},
        MergeChoice::KeepBoth{keep_a:true},&AtomicBool::new(false),|_|{}).unwrap();
    let kept=&report.preserved[0];
    assert_eq!(std::fs::read(ad.path().join(&kept.rel)).unwrap(),b"bravo");
    assert_eq!(std::fs::read(bd.path().join(&kept.rel)).unwrap(),b"bravo");
    assert_eq!(std::fs::read(&pa).unwrap(),b"alpha");
    assert_eq!(std::fs::read(&pb).unwrap(),b"alpha");
    assert_eq!(report.baseline[&kept.rel],(kept.a,kept.b));
    clean(&key);
}

#[test]
fn review_task_merge_pair_lock_precedes_original_observation() {
    let ad=tempfile::tempdir().unwrap(); let bd=tempfile::tempdir().unwrap();
    let ra=forward(ad.path()); let rb=forward(bd.path());
    let a=LocalBackend::new(&ra); let b=LocalBackend::new(&rb);
    let pa=format!("{ra}/f.txt"); let pb=format!("{rb}/f.txt");
    std::fs::write(&pa,b"alpha").unwrap(); std::fs::write(&pb,b"bravo").unwrap();
    let conflict=Conflict{rel:"f.txt".into(),a:Some(signature(&a,&pa)),b:Some(signature(&b,&pb)),duplicates:None};
    let key=state(&a,&ra,&b,&rb);
    let lock=PairLock::acquire(&key.lock_id).unwrap();
    let failure=merge_recorded_for_key(&a,&ra,&b,&rb,&key,&conflict,
        OriginalContent{signature:conflict.a,bytes:Some(b"alpha")},
        OriginalContent{signature:conflict.b,bytes:Some(b"bravo")},
        MergeChoice::Write(b"joined"),&AtomicBool::new(false),|_|{}).unwrap_err();
    assert_eq!(failure.error.kind(),io::ErrorKind::WouldBlock);
    assert_eq!(std::fs::read(&pa).unwrap(),b"alpha");
    assert_eq!(std::fs::read(&pb).unwrap(),b"bravo");
    drop(lock); clean(&key);
}
