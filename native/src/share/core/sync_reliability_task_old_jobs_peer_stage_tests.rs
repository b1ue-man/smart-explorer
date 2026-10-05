//! Generated client stages share the acknowledged creator proof, not discard rights.
use super::{Binding, StageLedger};
use crate::share::peer_writer::owned_writer;
use crate::vfs::VfsMeta;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

const STAGE: &str = "/Docs/file.se-daemon-0123456789abcdef";
const BYTES: &[u8] = b"acknowledged creator bytes";

fn binding() -> Binding {
    Binding::new("peer:Direct:c08:node".into(), Some("c08-lease".into()))
}

fn metadata(stage: &str) -> VfsMeta {
    VfsMeta {
        name: stage.rsplit('/').next().unwrap().into(),
        size: BYTES.len() as u64,
        mtime_ms: 42,
        id: Some("exclusively-created-id".into()),
        ..Default::default()
    }
}

struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    lose_ack: bool,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.lose_ack {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "WriteDone acknowledgement unavailable",
            ))
        } else {
            Ok(())
        }
    }
}

fn assert_exclusive_creator(stage: &str) {
    let ledger = StageLedger::default();
    let ticket = ledger.reserve(stage, binding()).unwrap();
    let creating = ledger.ready(stage, &binding()).err().unwrap();
    assert_eq!(creating.kind(), io::ErrorKind::PermissionDenied);
    assert!(creating.to_string().contains("state=creating"));
    ticket.opened().unwrap();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let mut writer = owned_writer(
        Box::new(RecordingWriter {
            bytes: bytes.clone(),
            lose_ack: false,
        }),
        ticket,
    );
    writer.write_all(BYTES).unwrap();
    let writing = ledger.ready(stage, &binding()).err().unwrap();
    assert_eq!(writing.kind(), io::ErrorKind::PermissionDenied);
    assert!(writing.to_string().contains("state=writing"));
    writer.flush().unwrap();
    drop(writer);
    ledger.verify(stage, &binding(), &metadata(stage)).unwrap();

    // Equal binding and path on a reopened backend are not a creator proof.
    let reopened = StageLedger::default();
    let foreign = reopened.ready(stage, &binding()).err().unwrap();
    assert_eq!(foreign.kind(), io::ErrorKind::PermissionDenied);
    assert!(foreign.to_string().contains("state=untracked"));
    assert!(foreign.to_string().contains(stage));
    assert!(!foreign.to_string().contains("c08-lease"));
    ledger.verify(stage, &binding(), &metadata(stage)).unwrap();

    let replacement = VfsMeta {
        id: Some("foreign-created-id".into()),
        ..metadata(stage)
    };
    assert_eq!(
        ledger
            .verify(stage, &binding(), &replacement)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(ledger.ready(stage, &binding()).is_err());
    assert!(ledger.contains(stage).unwrap());
    let retained = bytes.lock().unwrap().clone();
    assert_eq!(retained.as_slice(), BYTES);
}

fn assert_ack_and_binding_required(stage: &str) {
    for lose_ack in [true, false] {
        let ledger = StageLedger::default();
        let ticket = ledger.reserve(stage, binding()).unwrap();
        ticket.opened().unwrap();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let mut writer = owned_writer(
            Box::new(RecordingWriter {
                bytes: bytes.clone(),
                lose_ack,
            }),
            ticket,
        );
        writer.write_all(BYTES).unwrap();
        if lose_ack {
            assert_eq!(
                writer.flush().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
            assert_eq!(
                writer.flush().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        } else {
            writer.flush().unwrap();
            let rebound = Binding::new(
                "peer:Direct:c08:node".into(),
                Some("replacement-lease".into()),
            );
            assert_eq!(
                ledger.ready(stage, &rebound).err().unwrap().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        drop(writer);
        let uncertain = ledger.ready(stage, &binding()).err().unwrap();
        assert_eq!(uncertain.kind(), io::ErrorKind::PermissionDenied);
        assert!(uncertain.to_string().contains("state=pending"));
        assert!(ledger.reserve(stage, binding()).is_err());
        assert!(ledger.contains(stage).unwrap());
        let retained = bytes.lock().unwrap().clone();
        assert_eq!(retained.as_slice(), BYTES);
    }
}

#[test]
fn sync_reliability_task_old_jobs_daemon_stage_ack_keeps_exclusive_creator() {
    assert_exclusive_creator(STAGE);
}

#[test]
fn sync_reliability_task_old_jobs_daemon_stage_lost_ack_and_rebinding_stay_unproven() {
    assert_ack_and_binding_required(STAGE);
}

#[test]
fn sync_reliability_task_old_jobs_generated_purposes_keep_creator_and_ack_guards() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_string_lossy().replace('\\', "/");
    let backend = crate::vfs::LocalBackend::new(&root);
    // Existing callers, unseen purposes, and literal prefixes all use the
    // real generator. Eligibility never substitutes for the creator proof.
    for (name, purpose) in [
        ("file", "daemon"),
        (".se-sync-replica", "sync-replica"),
        ("file", "bisync"),
        ("file", "merge"),
        ("file", "transfer"),
        ("file", "copy"),
        ("file", "upload"),
        ("file", "future-7"),
        (".hidden", "0-next-9"),
        ("字.se-older-0123456789abcdef", "next-2"),
        ("file", "0"),
        ("file", "-"),
    ] {
        let destination = format!("{root}/{name}");
        let stage = crate::vfs::unique_staging_path(&backend, &destination, purpose).unwrap();
        assert_exclusive_creator(&stage);
        assert_ack_and_binding_required(&stage);
    }
}

fn assert_unrecognized(name: &str) {
    let path = format!("/Docs/{name}");
    let ledger = StageLedger::default();
    let error = ledger.reserve(&path, binding()).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("state=unrecognized-name"));
    assert!(!ledger.contains(&path).unwrap());
    assert!(ledger.ready(&path, &binding()).is_err());
}

#[test]
fn sync_reliability_task_old_jobs_generated_stage_names_reject_invalid_and_quarantine_shapes() {
    for name in [
        "plain.txt",
        ".se-sync-replica-0123456789abcdef",
        "file.se--0123456789abcdef",
        "file.se-Sync-replica-0123456789abcdef",
        "file.se-sync_replica-0123456789abcdef",
        "file.se-sync-replica-0123456789abcde",
        "file.se-sync-replica-0123456789abcdef0",
        "file.se-sync-replica-0123456789abcdeg",
        "file.se-sync-replica-0123456789abcdeF",
        "file.se-sync-replica-0123456789abcdef/",
    ] {
        assert_unrecognized(name);
    }
    // The broader scan filter is not a creator-eligibility or deletion proof.
    for name in [
        ".se-replace-0123456789abcdef",
        ".se-private-0123456789abcdef0123456789abcdef.tmp",
        ".file.smart-explorer.part",
        ".file.smart-explorer-a.move",
        "file.se-agent-batch-1-2.part",
        "file.se-upload-1-a.part",
        ".se-agent-tree-1-2-3.spool",
    ] {
        assert!(crate::vfs::is_staging_name(name), "{name}");
        assert_unrecognized(name);
    }
}
