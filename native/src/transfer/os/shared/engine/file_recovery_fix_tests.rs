//! Regression oracles for the reviewed P14 publication and optical gaps.
use super::super::super::{file_evidence, source::LocalSource, view::Side};
use super::*;
use crate::vfs::Backend;
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

fn receipt_files(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join("job.files"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap() != "index.json")
        .collect()
}
fn pending_index(root: &Path, id: &str, candidate: &Path, count: u64) {
    let receipt: serde_json::Value =
        read_record(candidate, RecordKind::Job, id, u64::MAX, false).unwrap();
    let path = root.join("job.files/index.json");
    let mut index: serde_json::Value =
        read_record(&path, RecordKind::Job, id, u64::MAX, false).unwrap();
    index["pending"] = receipt;
    index["count"] = count.into();
    write_record(&path, RecordKind::Job, id, None, &index).unwrap();
}

#[test]
fn batches_task_p14_pending_new_key_resumes_with_or_without_the_receipt_file() {
    for missing_candidate in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"alpha");
        fs::create_dir(&target).unwrap();
        let stage = target.join(".a.jpg.smart-explorer-0123456789abcdef.part");
        write(&stage, b"alpha");
        let job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            file_recovery::before(
                &engine,
                &file(&source, "a.jpg"),
                &fwd(&stage),
                &fwd(&target.join("a.jpg")),
            )
            .unwrap();
            let candidate = receipt_files(root.path()).pop().unwrap();
            pending_index(root.path(), &job.id, &candidate, 0);
            if missing_candidate {
                fs::remove_file(candidate).unwrap();
            }
        }
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        assert!(!stage.exists());
        assert_eq!(fs::read(&source).unwrap(), b"alpha");
        apply(&engine, file(&source, "a.jpg"));
        assert_eq!(engine.issues.total(), 0);
        assert_eq!(names(&target), ["a.jpg"]);
    }
}

#[test]
fn batches_task_p14_pending_intent_cannot_recount_a_lost_committed_receipt() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("a.jpg");
    let second = root.path().join("b.jpg");
    let target = root.path().join("target");
    write(&first, b"alpha");
    write(&second, b"beta");
    fs::create_dir(&target).unwrap();
    let stage = target.join(".b.jpg.smart-explorer-0123456789abcdef.part");
    write(&stage, b"beta");
    let job = job(
        Endpoint::Local,
        Endpoint::Local,
        &target,
        &[&first, &second],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, file(&first, "a.jpg"));
        let committed = receipt_files(root.path()).pop().unwrap();
        file_recovery::before(
            &engine,
            &file(&second, "b.jpg"),
            &fwd(&stage),
            &fwd(&target.join("b.jpg")),
        )
        .unwrap();
        let candidate = receipt_files(root.path())
            .into_iter()
            .find(|path| *path != committed)
            .unwrap();
        pending_index(root.path(), &job.id, &candidate, 1);
        fs::remove_file(committed).unwrap();
    }
    assert!(fixture(&job, root.path(), &cancel, &report).is_err());
    assert!(stage.exists());
    assert_eq!(fs::read(&second).unwrap(), b"beta");
}

#[test]
fn batches_task_p14_prepublication_old_target_is_not_a_foreign_postpublication_replacement() {
    for conflict in [Conflict::Rename, Conflict::Overwrite] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.jpg");
        let target = root.path().join("target");
        write(&source, b"new");
        write(&target.join("a.jpg"), b"old");
        let quarantine = root
            .path()
            .join(".a.jpg.smart-explorer-0123456789abcdef.move");
        let stage = target.join(".a.jpg.smart-explorer-0123456789abcdef.part");
        let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
        job.mode = CopyMode::Move;
        job.conflict = conflict;
        let cancel = AtomicBool::new(false);
        let report = |_| {};
        {
            let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
            let work = file(&source, "a.jpg");
            let observer = file_recovery::LocalObserver::new(&engine, &work);
            observer
                .before_source_quarantine(&source, &quarantine)
                .unwrap();
            fs::rename(&source, &quarantine).unwrap();
            fs::copy(&quarantine, &stage).unwrap();
            observer
                .before_publish(&stage, &target.join("a.jpg"))
                .unwrap();
        }
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        assert!(!quarantine.exists());
        assert!(!stage.exists());
        assert_eq!(fs::read(&source).unwrap(), b"new");
        assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"old");
        apply(&engine, file(&source, "a.jpg"));
        assert_eq!(engine.issues.total(), 0);
        assert_eq!(
            fs::read(target.join(if conflict == Conflict::Rename {
                "a (2).jpg"
            } else {
                "a.jpg"
            }))
            .unwrap(),
            b"new"
        );
    }
}

#[test]
fn batches_task_p14_restored_old_target_after_publication_preserves_quarantine() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"new");
    write(&target.join("a.jpg"), b"old");
    let quarantine = root
        .path()
        .join(".a.jpg.smart-explorer-0123456789abcdef.move");
    let stage = target.join(".a.jpg.smart-explorer-0123456789abcdef.part");
    let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    job.mode = CopyMode::Move;
    job.conflict = Conflict::Overwrite;
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        let work = file(&source, "a.jpg");
        let observer = file_recovery::LocalObserver::new(&engine, &work);
        observer
            .before_source_quarantine(&source, &quarantine)
            .unwrap();
        fs::rename(&source, &quarantine).unwrap();
        fs::copy(&quarantine, &stage).unwrap();
        observer
            .before_publish(&stage, &target.join("a.jpg"))
            .unwrap();
        fs::rename(target.join("a.jpg"), target.join("held-old")).unwrap();
        fs::rename(&stage, target.join("a.jpg")).unwrap();
        fs::rename(target.join("a.jpg"), target.join("held-new")).unwrap();
        fs::rename(target.join("held-old"), target.join("a.jpg")).unwrap();
    }
    assert!(fixture(&job, root.path(), &cancel, &report).is_err());
    assert_eq!(fs::read(&quarantine).unwrap(), b"new");
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"old");
}

#[test]
fn batches_task_p14_optical_hash_upload_and_media_handoff_have_one_permit_owner() {
    let root = tempfile::tempdir().unwrap();
    let disc = root.path().join("disc");
    let source = disc.join("a.bin");
    let target = root.path().join("target");
    write(
        &source,
        &vec![7; 2 * crate::source_media::OPTICAL_BLOCK_BYTES],
    );
    fs::create_dir(&target).unwrap();
    let medium = crate::source_media::Source {
        device: format!("p14-optical:{}", root.path().display()),
        generation: "a".into(),
        read_only: true,
    };
    let profile = crate::source_media::test_profiles::register(&disc, medium.clone());
    let job = job(
        Endpoint::Local,
        remote("optical-receipt", true),
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = (|| -> Result<(), String> {
                file_evidence::observe(Side::Local, &fwd(&source), &cancel)
                    .map_err(|error| error.to_string())?;
                let engine = fixture(&job, root.path(), &cancel, &report)
                    .map_err(|error| error.to_string())?;
                apply(&engine, file(&source, "a.bin"));
                if engine.issues.total() != 0 {
                    return Err(format!("{:?}", engine.issues.shown().0));
                }
                Ok(())
            })();
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(Duration::from_secs(10));
        if result.is_err() {
            cancel.store(true, Ordering::Release);
        }
        assert!(
            matches!(result, Ok(Ok(()))),
            "optical receipt/upload failed or nested its own permit: {result:?}"
        );
    });
    assert_eq!(
        fs::read(target.join("a.bin")).unwrap(),
        fs::read(&source).unwrap()
    );
    let check = LocalSource::open(&fwd(&source))
        .unwrap()
        .publication_check()
        .unwrap();
    profile.replace(crate::source_media::Source {
        generation: "b".into(),
        ..medium.clone()
    });
    assert!(check.verify().is_err());
    assert_eq!(
        crate::source_media::test_profiles::maximum(&medium.device),
        1
    );
}

#[test]
fn batches_task_p14_optical_evidence_wait_is_cancellable_before_read() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.bin");
    write(&source, b"alpha");
    let medium = crate::source_media::Source {
        device: format!("p14-cancel:{}", root.path().display()),
        generation: "a".into(),
        read_only: true,
    };
    let _profile = crate::source_media::test_profiles::register(root.path(), medium.clone());
    let cancel = AtomicBool::new(false);
    let held =
        crate::source_media::acquire(&source, crate::source_media::Priority::Foreground, &cancel)
            .unwrap()
            .unwrap();
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _ =
                tx.send(file_evidence::observe(Side::Local, &fwd(&source), &cancel).map(|_| ()));
        });
        let start = std::time::Instant::now();
        while !crate::source_media::snapshots()
            .iter()
            .any(|status| status.source.device == medium.device && status.foreground_waiters > 0)
        {
            if start.elapsed() > Duration::from_secs(10) {
                cancel.store(true, Ordering::Release);
                panic!("optical evidence worker never reached admission");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        cancel.store(true, Ordering::Release);
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(10)), Ok(Err(error)) if error.kind() == io::ErrorKind::Interrupted)
        );
    });
    drop(held);
    assert_eq!(
        crate::source_media::test_profiles::maximum(&medium.device),
        1
    );
}

struct ExportSource {
    local: crate::vfs::LocalBackend,
    regular_reads: std::sync::atomic::AtomicUsize,
}
impl crate::vfs::Backend for ExportSource {
    fn scheme(&self) -> crate::vfs::Scheme {
        crate::vfs::Scheme::Local
    }
    fn root_display(&self) -> String {
        "p14-export-source".into()
    }
    fn extensions(&self) -> Option<&dyn crate::vfs::BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> io::Result<Vec<crate::vfs::VfsMeta>> {
        self.local.list_dir(path)
    }
    fn stat(&self, path: &str) -> io::Result<crate::vfs::VfsMeta> {
        let mut meta = self.local.stat(path)?;
        meta.id = Some(format!("export:{path}"));
        meta.size = 0;
        Ok(meta)
    }
    fn open_read(&self, _: &str) -> io::Result<Box<dyn std::io::Read + Send>> {
        Err(io::Error::other("export needs the selected source ID"))
    }
    fn open_read_id(
        &self,
        path: &str,
        id: Option<&str>,
    ) -> io::Result<Box<dyn std::io::Read + Send>> {
        if id != Some(format!("export:{path}").as_str()) {
            return Err(io::Error::other("wrong export source ID"));
        }
        self.local.open_read(path)
    }
    fn read_size(&self, _: &str, _: u64) -> io::Result<Option<u64>> {
        Ok(None)
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn std::io::Write + Send>> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn remove_file(&self, _: &str) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn remove_dir(&self, _: &str) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
    fn mkdir_all(&self, _: &str) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }
}
impl crate::vfs::BackendExtensions for ExportSource {
    fn file_identity(&self, path: &str) -> io::Result<Option<String>> {
        Ok(Some(format!("export-object:{path}")))
    }
    fn open_read_regular(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> io::Result<Box<dyn std::io::Read + Send>> {
        self.regular_reads.fetch_add(1, Ordering::SeqCst);
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "regular read refuses Google-native objects",
        ))
    }
}

#[test]
fn batches_task_p14_export_source_reopen_uses_its_selected_id_and_export_reader() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("document");
    let target = root.path().join("target");
    write(&source, b"%PDF exported alpha");
    fs::create_dir(&target).unwrap();
    let backend = std::sync::Arc::new(ExportSource {
        local: crate::vfs::LocalBackend::new("/"),
        regular_reads: std::sync::atomic::AtomicUsize::new(0),
    });
    let job = job(
        Endpoint::Remote(backend.clone()),
        Endpoint::Local,
        &target,
        &[&source],
    );
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let meta = backend.stat(&fwd(&source)).unwrap();
    let mut work = FileWork::new(
        fwd(&source),
        "document.pdf".into(),
        meta.size,
        meta.mtime_ms,
    );
    work.id = meta.id;
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        apply(&engine, work.clone());
        assert_eq!(engine.issues.total(), 0);
    }
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    apply(&engine, work);
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(names(&target), ["document.pdf"]);
    assert_eq!(
        fs::read(target.join("document.pdf")).unwrap(),
        b"%PDF exported alpha"
    );
    assert_eq!(backend.regular_reads.load(Ordering::SeqCst), 0);
}

#[test]
fn batches_task_p14_failed_prepare_acknowledgement_recovers_after_stage_discard() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"new");
    write(&target.join("a.jpg"), b"old");
    let quarantine = root
        .path()
        .join(".a.jpg.smart-explorer-0123456789abcdef.move");
    let stage = target.join(".a.jpg.smart-explorer-0123456789abcdef.part");
    let mut job = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    job.mode = CopyMode::Move;
    job.conflict = Conflict::Overwrite;
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
        let work = file(&source, "a.jpg");
        let observer = file_recovery::LocalObserver::new(&engine, &work);
        observer
            .before_source_quarantine(&source, &quarantine)
            .unwrap();
        fs::rename(&source, &quarantine).unwrap();
        fs::copy(&quarantine, &stage).unwrap();
        observer
            .before_publish(&stage, &target.join("a.jpg"))
            .unwrap();
        // The journal replacement reached disk but its final durability
        // acknowledgement failed, so Copy never called the publication.
        observer
            .publication_preparation_failed(&stage, &target.join("a.jpg"))
            .unwrap();
        fs::remove_file(&stage).unwrap();
    }
    let engine = fixture(&job, root.path(), &cancel, &report).unwrap();
    assert!(!quarantine.exists());
    assert_eq!(fs::read(&source).unwrap(), b"new");
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"old");
    apply(&engine, file(&source, "a.jpg"));
    assert_eq!(engine.issues.total(), 0);
    assert_eq!(fs::read(target.join("a.jpg")).unwrap(), b"new");
}
