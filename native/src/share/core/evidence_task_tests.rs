//! Loopback coverage of `evidence_v1` and `create_dirs_v1` (K-B, K-F): what
//! the host reports in its replies equals what separate `stat` and
//! `file_identity` requests answer, a refused `prepare` publishes nothing,
//! and folders are created in bulk with their "created" flags.
use std::fs;
use std::io::{self, Read, Write};

use super::transfer_engine_task_support::{get, put, stages_after_cleanup};
use super::CopyPastePeerFixture;
use crate::vfs::{Backend, BatchSink, EvidencePutOutcome, FileEvidence};

/// The evidence a client would have collected with its own requests.
fn asked(backend: &dyn Backend, path: &str) -> io::Result<FileEvidence> {
    let meta = backend.stat(path)?;
    Ok(FileEvidence {
        path: path.to_string(),
        object: crate::vfs::file_identity(backend, path)?,
        id: meta.id,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
    })
}

#[test]
fn speed_task_share_put_batch_evidence_equals_separate_requests() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &*fixture.backend;
    assert!(crate::vfs::transfer_evidence(backend, "/B"));
    fs::write(fixture.root_b.join("taken.txt"), b"old")?;
    let entries = [put("/B/taken.txt", 3), put("/B/fresh.txt", 5)];
    let mut seen = Vec::new();
    let mut prepare = |offset: usize, existed: &[Option<bool>]| -> io::Result<()> {
        seen.push((offset, existed.to_vec()));
        Ok(())
    };
    let outcomes =
        crate::vfs::put_batch_evidence(backend, &entries, &mut &b"newfresh"[..], &mut prepare)?;
    // Presence is reported before the commit, per requested path.
    assert_eq!(seen, vec![(0, vec![Some(true), Some(false)])]);
    let mut published = Vec::new();
    for outcome in outcomes {
        match outcome {
            EvidencePutOutcome::Published {
                path,
                evidence: Some(evidence),
            } => {
                assert_eq!(evidence, asked(backend, &path)?, "{path}");
                assert!(evidence.object.is_some(), "{path}");
                published.push(path);
            }
            other => panic!("published with evidence expected: {other:?}"),
        }
    }
    // Keep both: the taken name is numbered, never replaced.
    assert_eq!(published, ["/B/taken (2).txt", "/B/fresh.txt"]);
    assert_eq!(fs::read(fixture.root_b.join("taken.txt"))?, b"old");
    assert_eq!(fs::read(fixture.root_b.join("taken (2).txt"))?, b"new");

    // A refused prepare (the intent could not be written) publishes nothing,
    // and the failure says so.
    let mut refuse = |_: usize, _: &[Option<bool>]| -> io::Result<()> {
        Err(io::Error::other("fixture: no intent"))
    };
    let outcomes = crate::vfs::put_batch_evidence(
        backend,
        &[put("/B/never.txt", 2)],
        &mut &b"no"[..],
        &mut refuse,
    )?;
    match &outcomes[..] {
        [EvidencePutOutcome::Failed(error)] => assert!(crate::vfs::unpublished_of(error)),
        other => panic!("an unpublished failure expected: {other:?}"),
    }
    assert!(!fixture.root_b.join("never.txt").exists());
    assert_eq!(stages_after_cleanup(&fixture.root_b)?, Vec::<String>::new());
    Ok(())
}

#[derive(Default)]
struct EvidenceSink {
    evidence: Vec<(usize, FileEvidence)>,
    bytes: Vec<u8>,
}

impl BatchSink for EvidenceSink {
    fn begin(&mut self, _index: usize, _size: u64) -> io::Result<()> {
        Ok(())
    }

    fn data(&mut self, _index: usize, bytes: &[u8]) -> io::Result<()> {
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn end(&mut self, _index: usize, result: io::Result<()>) -> io::Result<()> {
        result
    }

    fn failed(&mut self, _index: usize, error: io::Error) -> io::Result<()> {
        Err(error)
    }

    fn evidence(&mut self, index: usize, evidence: FileEvidence) -> io::Result<()> {
        self.evidence.push((index, evidence));
        Ok(())
    }
}

#[test]
fn speed_task_share_read_evidence_equals_separate_requests() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &*fixture.backend;
    fs::write(fixture.root_a.join("one.txt"), b"one")?;
    fs::write(fixture.root_a.join("two.txt"), b"two!")?;

    let mut sink = EvidenceSink::default();
    backend.get_batch(&[get("/A/one.txt", 3), get("/A/two.txt", 4)], &mut sink)?;
    assert_eq!(sink.bytes, b"onetwo!");
    assert_eq!(
        sink.evidence,
        vec![
            (0, asked(backend, "/A/one.txt")?),
            (1, asked(backend, "/A/two.txt")?)
        ]
    );

    let (mut reader, evidence) =
        crate::vfs::open_read_evidence(backend, "/A/two.txt", None)?.expect("read evidence");
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    assert_eq!(text, "two!");
    assert_eq!(evidence, asked(backend, "/A/two.txt")?);
    Ok(())
}

#[test]
fn speed_task_share_stage_and_promote_evidence() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &*fixture.backend;
    let stage = "/B/up.txt.se-upload-00000000000000c3";
    let mut writer = backend.open_write_copy_stage_sized(stage, 2)?;
    writer.write_all(b"up")?;
    writer.flush()?;
    drop(writer);
    let staged = crate::vfs::take_stage_evidence(backend, stage).expect("stage evidence");
    assert_eq!(staged.destination.as_deref(), Some("/B/up.txt"));
    assert_eq!(staged.existed, Some(false));
    assert_eq!(staged.stage, asked(backend, stage)?);
    // Taken once.
    assert!(crate::vfs::take_stage_evidence(backend, stage).is_none());

    let published = crate::vfs::promote_copy_stage_evidence(backend, stage, "/B/up.txt")?
        .expect("promote evidence");
    assert_eq!(published, asked(backend, "/B/up.txt")?);
    assert_eq!(fs::read(fixture.root_b.join("up.txt"))?, b"up");
    // No replacement: a second stage for the same name is refused.
    let second = "/B/up.txt.se-upload-00000000000000c4";
    let mut writer = backend.open_write_copy_stage_sized(second, 3)?;
    writer.write_all(b"two")?;
    writer.flush()?;
    drop(writer);
    let staged = crate::vfs::take_stage_evidence(backend, second).expect("stage evidence");
    assert_eq!(staged.existed, Some(true));
    let refused = crate::vfs::promote_copy_stage_evidence(backend, second, "/B/up.txt");
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(fixture.root_b.join("up.txt"))?, b"up");
    Ok(())
}

#[test]
fn speed_task_share_create_dirs_reports_created_folders() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let backend = &*fixture.backend;
    assert!(crate::vfs::supports_create_dirs(backend, "/B"));
    assert!(!crate::vfs::supports_create_dirs(backend, "/"));
    fs::create_dir(fixture.root_b.join("old"))?;
    fs::write(fixture.root_b.join("file"), b"x")?;
    let paths: Vec<String> = ["/B/old", "/B/old/new", "/B/n", "/B/n/m", "/B/file"]
        .iter()
        .map(|path| path.to_string())
        .collect();
    let results = crate::vfs::create_dirs(backend, &paths)?;
    let created: Vec<Option<bool>> = results.iter().map(|r| r.as_ref().ok().copied()).collect();
    assert_eq!(
        created,
        [Some(false), Some(true), Some(true), Some(true), None],
        "{results:?}"
    );
    assert!(fixture.root_b.join("n/m").is_dir());
    assert!(fixture.root_b.join("file").is_file());
    Ok(())
}
