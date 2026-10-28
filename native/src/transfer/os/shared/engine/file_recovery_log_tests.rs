//! Crash log v2 (K-A, 2026-10-28): what a resumed job finds after a crash
//! at each point of a publication, a torn last record, the conversion of a
//! job directory of 0.5.188, and one record for a whole packet's intents.
use super::super::{
    ops::COPY_BUFFER,
    roots::RootPlan,
    side_flow,
    test_backend::{fwd, job, write},
    worker::run_file,
    FolderRegister,
};
use super::*;
use crate::transfer::{Endpoint, RecoveryMode, TransferJob, TransferMsg};
use std::fs;

fn open<'a>(
    candidate: &'a TransferJob,
    cancel: &'a AtomicBool,
    report: &'a (dyn Fn(TransferMsg) + Sync),
) -> Engine<'a> {
    let view = JobView::of(candidate);
    let record = super::super::job_store::begin(&view, view.target_dir, cancel).unwrap();
    let folders = FolderRegister::new(
        view.target,
        view.target_dir,
        side_flow(view.target, view.target_dir),
    );
    let mut engine = Engine::new(
        view,
        view.kind(),
        folders,
        cancel,
        report,
        &RootPlan::default(),
    );
    engine.job_record = record;
    engine
}

fn work(path: &Path, rel: &str) -> FileWork {
    let meta = fs::metadata(path).unwrap();
    FileWork::new(
        fwd(path),
        rel.into(),
        meta.len(),
        crate::local_access::system_time_ms(meta.modified().unwrap()),
    )
}

fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn job_path(id: &str) -> PathBuf {
    crate::support_dirs::app_data_file("transfer-jobs").join(format!("{id}.json"))
}

fn apply(engine: &Engine<'_>, file: FileWork) {
    run_file(engine, file, &mut [0; COPY_BUFFER]);
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
}

/// A source, an empty target folder and the private stage a crash left.
fn layout(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let source = root.join("a.jpg");
    let target = root.join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let stage = target.join(".a.jpg.smart-explorer-0123456789abcdef.part");
    write(&stage, b"alpha");
    (source, target, stage)
}

#[test]
fn speed_task_crash_log_intent_without_publication_discards_the_stage() {
    let root = tempfile::tempdir().unwrap();
    let (source, target, stage) = layout(root.path());
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        let file = work(&source, "a.jpg");
        before(&engine, &file, &fwd(&stage), &fwd(&target.join("a.jpg"))).unwrap();
        // The crash comes before the rename.
    }
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    assert!(!stage.exists(), "the crash's own stage is removed");
    assert_eq!(fs::read(&source).unwrap(), b"alpha");
    apply(&engine, work(&source, "a.jpg"));
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn speed_task_crash_log_publication_without_acknowledgement_is_decided_by_metadata() {
    let root = tempfile::tempdir().unwrap();
    let (source, target, stage) = layout(root.path());
    let destination = target.join("a.jpg");
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        let file = work(&source, "a.jpg");
        before(&engine, &file, &fwd(&stage), &fwd(&destination)).unwrap();
        // Published; the crash comes before the acknowledgement.
        fs::rename(&stage, &destination).unwrap();
    }
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    let receipt = engine
        .job_record
        .as_ref()
        .unwrap()
        .read_file(&work(&source, "a.jpg"))
        .unwrap()
        .unwrap();
    assert!(!receipt.acknowledged && receipt.publication_authorized);
    // The stage's object renamed into place is this job's own: kept, no
    // second copy under a numbered name.
    apply(&engine, work(&source, "a.jpg"));
    assert_eq!(names(&target), ["a.jpg"]);
    assert_eq!(fs::read(&destination).unwrap(), b"alpha");
}

#[test]
fn speed_task_crash_log_torn_last_record_is_cut_and_the_file_stays_in_doubt() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        apply(&engine, work(&source, "a.jpg"));
    }
    // The acknowledgement is the last record: tear it as a crash during its
    // write would.
    let log = log_path(&job_path(&candidate.id));
    let length = fs::metadata(&log).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&log)
        .unwrap()
        .set_len(length - 3)
        .unwrap();
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    assert!(
        fs::metadata(&log).unwrap().len() < length - 3,
        "the torn record is cut off"
    );
    let receipt = engine
        .job_record
        .as_ref()
        .unwrap()
        .read_file(&work(&source, "a.jpg"))
        .unwrap()
        .unwrap();
    assert!(!receipt.acknowledged, "only the durable intent is left");
    apply(&engine, work(&source, "a.jpg"));
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn speed_task_crash_log_converts_an_old_format_job_directory() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("a.jpg");
    let target = root.path().join("target");
    write(&source, b"alpha");
    fs::create_dir(&target).unwrap();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        apply(&engine, work(&source, "a.jpg"));
    }
    let path = job_path(&candidate.id);
    let data: JobRecord =
        read_record(&path, RecordKind::Job, &candidate.id, u64::MAX, false).unwrap();
    convert_to_old_format(&path, &data, None).unwrap();
    let folder = path.with_extension("files");
    assert!(folder.join("index.json").exists());
    assert!(!log_path(&path).exists());
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    assert!(log_path(&path).exists(), "converted into the crash log");
    assert!(!folder.join("index.json").exists(), "older files removed");
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    // The acknowledged receipt survived the conversion: kept, not copied again.
    apply(&engine, work(&source, "a.jpg"));
    assert_eq!(names(&target), ["a.jpg"]);
}

#[test]
fn speed_task_crash_log_packet_intents_are_one_record_and_host_evidence_acknowledges() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    fs::create_dir(&target).unwrap();
    let sources: Vec<PathBuf> = (0..3)
        .map(|index| root.path().join(format!("f{index}.txt")))
        .collect();
    for source in &sources {
        write(source, b"tiny");
    }
    let selected: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let candidate = job(Endpoint::Local, Endpoint::Local, &target, &selected);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        let works: Vec<FileWork> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| work(source, &format!("f{index}.txt")))
            .collect();
        let members: Vec<PacketIntent<'_>> = works
            .iter()
            .map(|file| PacketIntent {
                file,
                source: None,
                existed_before: Some(false),
            })
            .collect();
        prepare_batch_known(&engine, &members).unwrap();
        let record = engine.job_record.as_ref().unwrap();
        for file in &works {
            let receipt = record.read_file(file).unwrap().unwrap();
            assert!(receipt.publication_authorized && !receipt.acknowledged);
            assert_eq!(receipt.previous, Some(PreviousTarget::Absent));
        }
        // The host's reply named the published target: acknowledged from it.
        let destination = target.join("f0.txt");
        write(&destination, b"tiny");
        let evidence = HostEvidence {
            path: fwd(&destination),
            object: Some("host-object".into()),
            id: None,
            size: 4,
            mtime_ms: 1,
        };
        after_known(&engine, &works[0], &fwd(&destination), Some(&evidence)).unwrap();
        let receipt = record.read_file(&works[0]).unwrap().unwrap();
        assert!(receipt.acknowledged);
        assert_eq!(receipt.published, Some(evidence.proof()));
    }
    let mut records = 0;
    drop(
        crash_log::Log::open(&log_path(&job_path(&candidate.id)), |_, _| {
            records += 1;
            Ok(())
        })
        .unwrap(),
    );
    assert_eq!(records, 3, "header, one packet intent, one acknowledgement");
}
